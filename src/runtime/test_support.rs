use std::time::Duration;

use crate::runtime::transport::{
    ActivityNotifier, HidChannel, SendAcceptance, TransportEvent, TransportPort, TransportResult,
    fake::{FakeTransport, FakeTransportControl},
};

pub(crate) struct TestTransport {
    inner: FakeTransport,
}

#[derive(Clone)]
pub(crate) struct TestTransportControl {
    inner: FakeTransportControl,
}

impl TestTransport {
    pub(crate) fn with_limits(
        event_capacity: usize,
        max_poll_batch: usize,
    ) -> (Self, TestTransportControl) {
        let (inner, control) = FakeTransport::with_limits(event_capacity, max_poll_batch);
        (Self { inner }, TestTransportControl { inner: control })
    }
}

impl TransportPort for TestTransport {
    fn open(&mut self, activity: ActivityNotifier) -> TransportResult<()> {
        self.inner.open(activity)
    }

    fn poll(&mut self, timeout: Duration) -> TransportResult<Vec<TransportEvent>> {
        self.inner.poll(timeout)
    }

    fn send_interrupt(&mut self, payload: &[u8]) -> TransportResult<SendAcceptance> {
        self.inner.send_interrupt(payload)
    }

    fn drain_interrupt(&mut self, timeout: Duration) -> TransportResult<()> {
        self.inner.drain_interrupt(timeout)
    }

    fn disconnect(&mut self) -> TransportResult<()> {
        self.inner.disconnect()
    }

    fn close(&mut self) -> TransportResult<()> {
        self.inner.close()
    }
}

impl TestTransportControl {
    pub(crate) fn inject_connected(&self) -> TransportResult<()> {
        self.inner.inject_connected()
    }

    pub(crate) fn inject_hid_channel_opened(&self, channel: HidChannel) -> TransportResult<()> {
        self.inner.inject_hid_channel_opened(channel)
    }

    pub(crate) fn inject_hid_output(
        &self,
        channel: HidChannel,
        payload: &[u8],
    ) -> TransportResult<()> {
        self.inner.inject_hid_output(channel, payload)
    }

    pub(crate) fn inject_disconnected(&self, reason: Option<u8>) -> TransportResult<()> {
        self.inner.inject_disconnected(reason)
    }

    pub(crate) fn counters(&self) -> (usize, usize, usize) {
        let counters = self.inner.counters();
        (counters.open, counters.disconnect, counters.close)
    }
}
