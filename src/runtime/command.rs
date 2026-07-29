use std::{
    collections::VecDeque,
    sync::mpsc::{Receiver, SyncSender, TryRecvError, TrySendError, sync_channel},
};

use super::{
    transport::ActivityNotifier,
    worker::{CommandSource, StepProgress, WorkerCommandError, WorkerCommandProgress},
};

pub(crate) type CommandResult = Result<(), WorkerCommandError>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CommandEnqueueError {
    Busy,
    Disconnected,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "T26 blocking controller calls map a disconnected response to WorkerFailed"
    )
)]
pub(crate) enum CommandResponseError {
    WorkerFailed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "T24 worker loop handles response delivery invariant failures"
    )
)]
pub(crate) enum CommandDeliveryError {
    MissingResponse,
    ResponseBufferFull,
}

struct CommandRequest<C> {
    command: C,
    completion: CommandCompletion,
}

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "T24 worker loop owns each dequeued command completion"
    )
)]
struct CommandCompletion {
    sender: SyncSender<CommandResult>,
}

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "T24 worker loop delivers each command result exactly once"
    )
)]
impl CommandCompletion {
    fn respond(self, result: CommandResult) -> Result<(), CommandDeliveryError> {
        match self.sender.try_send(result) {
            Ok(()) | Err(TrySendError::Disconnected(_)) => Ok(()),
            Err(TrySendError::Full(_)) => Err(CommandDeliveryError::ResponseBufferFull),
        }
    }
}

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "T24 controller worker construction owns the command client"
    )
)]
pub(crate) struct CommandClient<C> {
    sender: SyncSender<CommandRequest<C>>,
    activity: ActivityNotifier,
}

impl<C> CommandClient<C> {
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "T24 controller worker construction submits typed commands"
        )
    )]
    pub(crate) fn try_enqueue(&self, command: C) -> Result<CommandResponse, CommandEnqueueError> {
        let (response_sender, response) = sync_channel(1);
        let request = CommandRequest {
            command,
            completion: CommandCompletion {
                sender: response_sender,
            },
        };
        match self.sender.try_send(request) {
            Ok(()) => {
                self.activity.notify();
                Ok(CommandResponse { receiver: response })
            }
            Err(TrySendError::Full(_)) => Err(CommandEnqueueError::Busy),
            Err(TrySendError::Disconnected(_)) => Err(CommandEnqueueError::Disconnected),
        }
    }
}

pub(crate) struct CommandReceiver<C> {
    receiver: Receiver<CommandRequest<C>>,
    in_flight: VecDeque<CommandCompletion>,
}

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "T24 worker loop delivers deterministic step results"
    )
)]
impl<C> CommandReceiver<C> {
    pub(crate) fn deliver_progress(
        &mut self,
        progress: &mut StepProgress,
    ) -> Result<(), CommandDeliveryError> {
        self.deliver(progress.take_command_results())
    }

    pub(crate) fn deliver_completion(
        &mut self,
        result: CommandResult,
    ) -> Result<(), CommandDeliveryError> {
        self.deliver([WorkerCommandProgress::Complete(result)])
    }

    fn deliver(
        &mut self,
        results: impl IntoIterator<Item = WorkerCommandProgress>,
    ) -> Result<(), CommandDeliveryError> {
        for result in results {
            match result {
                WorkerCommandProgress::Pending => {
                    if self.in_flight.front().is_none() {
                        return Err(CommandDeliveryError::MissingResponse);
                    }
                }
                WorkerCommandProgress::Complete(result) => {
                    let Some(completion) = self.in_flight.pop_front() else {
                        return Err(CommandDeliveryError::MissingResponse);
                    };
                    completion.respond(result)?;
                }
            }
        }
        Ok(())
    }
}

impl<C> CommandSource<C> for CommandReceiver<C> {
    fn try_next(&mut self) -> Option<C> {
        match self.receiver.try_recv() {
            Ok(request) => {
                self.in_flight.push_back(request.completion);
                Some(request.command)
            }
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => None,
        }
    }
}

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "T24 public command calls wait on the one-shot response"
    )
)]
pub(crate) struct CommandResponse {
    receiver: Receiver<CommandResult>,
}

impl CommandResponse {
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "T26 blocking controller calls wait for a response or worker termination"
        )
    )]
    pub(crate) fn recv(self) -> Result<CommandResult, CommandResponseError> {
        self.receiver
            .recv()
            .map_err(|_| CommandResponseError::WorkerFailed)
    }

    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "T24 public command calls wait on the one-shot response"
        )
    )]
    pub(crate) fn try_recv(&self) -> Result<CommandResult, TryRecvError> {
        self.receiver.try_recv()
    }
}

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "T24 controller worker construction chooses the measured queue capacity"
    )
)]
pub(crate) fn command_channel<C>(
    capacity: usize,
    activity: ActivityNotifier,
) -> (CommandClient<C>, CommandReceiver<C>) {
    assert!(capacity > 0, "worker command capacity must be positive");
    let (sender, receiver) = sync_channel(capacity);
    (
        CommandClient { sender, activity },
        CommandReceiver {
            receiver,
            in_flight: VecDeque::new(),
        },
    )
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::TryRecvError;

    use crate::runtime::{
        transport::activity_channel,
        worker::{CommandSource, WorkerCommandError, WorkerCommandProgress},
    };

    use super::{CommandDeliveryError, CommandEnqueueError, command_channel};

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum TestCommand {
        First,
        Second,
        Third,
    }

    #[test]
    fn full_queue_is_busy_and_only_successful_enqueue_wakes() {
        let (activity, wakes) = activity_channel();
        let (client, mut worker) = command_channel(1, activity);

        let first = client
            .try_enqueue(TestCommand::First)
            .expect("first command fits");
        wakes.try_recv().expect("accepted command wakes worker");
        assert!(matches!(
            client.try_enqueue(TestCommand::Second),
            Err(CommandEnqueueError::Busy)
        ));
        assert_eq!(wakes.try_recv(), Err(TryRecvError::Empty));
        assert_eq!(worker.try_next(), Some(TestCommand::First));
        assert!(matches!(first.try_recv(), Err(TryRecvError::Empty)));

        worker
            .deliver([WorkerCommandProgress::Complete(Ok(()))])
            .expect("deliver first completion");
        assert!(matches!(first.try_recv(), Ok(Ok(()))));

        let _third = client
            .try_enqueue(TestCommand::Third)
            .expect("draining the queue frees capacity");
        wakes.try_recv().expect("new accepted command rearms wake");
        assert_eq!(worker.try_next(), Some(TestCommand::Third));
    }

    #[test]
    fn receiver_disconnect_is_not_reported_as_busy_or_woken() {
        let (activity, wakes) = activity_channel();
        let (client, worker) = command_channel(1, activity);
        let _queued = client
            .try_enqueue(TestCommand::First)
            .expect("prefill command queue");
        wakes.try_recv().expect("prefill wake");
        drop(worker);

        assert!(matches!(
            client.try_enqueue(TestCommand::Second),
            Err(CommandEnqueueError::Disconnected)
        ));
        assert_eq!(wakes.try_recv(), Err(TryRecvError::Empty));
    }

    #[test]
    fn pending_command_keeps_its_response_until_typed_completion() {
        let (activity, _wakes) = activity_channel();
        let (client, mut worker) = command_channel(1, activity);
        let response = client
            .try_enqueue(TestCommand::First)
            .expect("enqueue pending command");
        assert_eq!(worker.try_next(), Some(TestCommand::First));

        worker
            .deliver([WorkerCommandProgress::Pending])
            .expect("pending retains its response");
        assert!(matches!(response.try_recv(), Err(TryRecvError::Empty)));
        worker
            .deliver([WorkerCommandProgress::Complete(Err(
                WorkerCommandError::Shutdown,
            ))])
            .expect("typed completion consumes the response");
        assert!(matches!(
            response.try_recv(),
            Ok(Err(WorkerCommandError::Shutdown))
        ));

        let abandoned = client
            .try_enqueue(TestCommand::Second)
            .expect("enqueue abandoned response");
        assert_eq!(worker.try_next(), Some(TestCommand::Second));
        drop(abandoned);
        worker
            .deliver([WorkerCommandProgress::Complete(Ok(()))])
            .expect("a dropped caller does not fail worker delivery");

        let same_step = client
            .try_enqueue(TestCommand::Third)
            .expect("enqueue zero-duration style command");
        assert_eq!(worker.try_next(), Some(TestCommand::Third));
        worker
            .deliver([
                WorkerCommandProgress::Pending,
                WorkerCommandProgress::Complete(Ok(())),
            ])
            .expect("same-step completion consumes the retained response once");
        assert!(matches!(same_step.try_recv(), Ok(Ok(()))));
    }

    #[test]
    fn fifo_commands_keep_their_own_one_shot_responses() {
        let (activity, wakes) = activity_channel();
        let (client, mut worker) = command_channel(2, activity);
        let first = client
            .try_enqueue(TestCommand::First)
            .expect("enqueue first");
        let second = client
            .try_enqueue(TestCommand::Second)
            .expect("enqueue second");
        wakes.try_recv().expect("coalesced command wake");
        assert_eq!(wakes.try_recv(), Err(TryRecvError::Empty));

        assert_eq!(worker.try_next(), Some(TestCommand::First));
        assert_eq!(worker.try_next(), Some(TestCommand::Second));
        worker
            .deliver([
                WorkerCommandProgress::Complete(Ok(())),
                WorkerCommandProgress::Complete(Err(WorkerCommandError::Shutdown)),
            ])
            .expect("deliver FIFO completions");

        assert!(matches!(first.try_recv(), Ok(Ok(()))));
        assert!(matches!(
            second.try_recv(),
            Ok(Err(WorkerCommandError::Shutdown))
        ));
        assert_eq!(
            worker.deliver([WorkerCommandProgress::Complete(Ok(()))]),
            Err(CommandDeliveryError::MissingResponse)
        );
    }
}
