const BITS_PER_WORD: usize = u64::BITS as usize;
const WORD_COUNT: usize = 256 / BITS_PER_WORD;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ObservedSubcommands {
    words: [u64; WORD_COUNT],
    last: Option<u8>,
}

impl ObservedSubcommands {
    pub(crate) fn observe(&mut self, id: u8) -> bool {
        self.last = Some(id);
        let (word, mask) = bit_slot(id);
        let first_observation = self.words[word] & mask == 0;
        self.words[word] |= mask;
        first_observation
    }

    #[must_use]
    pub(crate) fn is_empty(&self) -> bool {
        self.words.iter().all(|word| *word == 0)
    }

    #[must_use]
    pub(crate) const fn last(&self) -> Option<u8> {
        self.last
    }

    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }
}

fn bit_slot(id: u8) -> (usize, u64) {
    let index = usize::from(id);
    (index / BITS_PER_WORD, 1_u64 << (index % BITS_PER_WORD))
}

#[cfg(test)]
mod tests {
    use super::ObservedSubcommands;

    #[test]
    fn observations_deduplicate_ids_across_the_full_u8_space() {
        let mut observed = ObservedSubcommands::default();

        assert!(observed.is_empty());
        for id in u8::MIN..=u8::MAX {
            assert!(observed.observe(id), "first observation of 0x{id:02x}");
        }
        assert!(!observed.is_empty());
        for id in u8::MIN..=u8::MAX {
            assert!(!observed.observe(id), "duplicate observation of 0x{id:02x}");
        }
    }

    #[test]
    fn reset_starts_a_new_observation_lifetime_without_a_stale_id() {
        let mut observed = ObservedSubcommands::default();
        assert!(observed.observe(0x40));
        assert!(observed.observe(0x03));

        observed.reset();

        assert!(observed.is_empty());
        assert_eq!(observed.last(), None);
        assert!(observed.observe(0x40));
        assert!(observed.observe(0x03));
        assert_eq!(observed.last(), Some(0x03));
    }
}
