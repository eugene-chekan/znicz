//! Moves decoded samples into the output without ever losing any.
//!
//! The ring buffer is small (about two seconds). Decoding is much faster than
//! playback, so the buffer fills up quickly. When that happens a packet only
//! fits partly. The leftover part MUST be kept for the next round: dropping it
//! makes playback skip forward, which sounds like fast-forward.

/// Anything we can write interleaved samples into.
pub trait SampleSink {
    /// How many samples can be written right now.
    fn write_slots(&self) -> usize;
    /// Write as much as fits. Returns how many samples were accepted.
    fn push_samples(&mut self, samples: &[f32]) -> usize;
}

/// One step of decoding.
pub enum DecodeStep {
    Samples(Vec<f32>),
    End,
    Failed(String),
}

#[derive(Debug, PartialEq, Eq)]
pub enum PumpOutcome {
    /// Output is full for now. Call again later.
    SinkFull,
    /// The track finished.
    Finished,
    /// The decoder failed.
    Failed(String),
}

/// Keeps the leftover of a partly written packet.
#[derive(Default)]
pub struct Feeder {
    pending: Vec<f32>,
    offset: usize,
    pushed: u64,
}

impl Feeder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Forget queued audio (used on seek, stop and track change).
    pub fn reset(&mut self) {
        self.pending.clear();
        self.offset = 0;
        self.pushed = 0;
    }

    /// Total samples handed to the output since the last reset.
    pub fn pushed_samples(&self) -> u64 {
        self.pushed
    }

    pub fn set_pushed_samples(&mut self, samples: u64) {
        self.pushed = samples;
    }

    pub fn pending_samples(&self) -> usize {
        self.pending.len().saturating_sub(self.offset)
    }

    /// Write leftover audio, then decode more while there is room.
    ///
    /// `max_packets` bounds how long one call may run so the player loop keeps
    /// answering commands.
    pub fn pump<S: SampleSink>(
        &mut self,
        sink: &mut S,
        min_slots: usize,
        max_packets: usize,
        decode: &mut dyn FnMut() -> DecodeStep,
    ) -> PumpOutcome {
        if !self.drain(sink) {
            return PumpOutcome::SinkFull;
        }

        for _ in 0..max_packets {
            if sink.write_slots() < min_slots {
                return PumpOutcome::SinkFull;
            }

            match decode() {
                DecodeStep::Samples(samples) => {
                    if samples.is_empty() {
                        continue;
                    }
                    self.pending = samples;
                    self.offset = 0;
                    if !self.drain(sink) {
                        return PumpOutcome::SinkFull;
                    }
                }
                DecodeStep::End => return PumpOutcome::Finished,
                DecodeStep::Failed(message) => return PumpOutcome::Failed(message),
            }
        }

        PumpOutcome::SinkFull
    }

    /// Returns true when nothing is left over.
    fn drain<S: SampleSink>(&mut self, sink: &mut S) -> bool {
        if self.offset >= self.pending.len() {
            self.pending.clear();
            self.offset = 0;
            return true;
        }

        let written = sink.push_samples(&self.pending[self.offset..]);
        self.offset += written;
        self.pushed += written as u64;

        if self.offset >= self.pending.len() {
            self.pending.clear();
            self.offset = 0;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sink with a fixed capacity that can be drained, like the real ring.
    struct FakeSink {
        capacity: usize,
        queued: Vec<f32>,
        played: Vec<f32>,
    }

    impl FakeSink {
        fn new(capacity: usize) -> Self {
            Self {
                capacity,
                queued: Vec::new(),
                played: Vec::new(),
            }
        }

        /// Pretend the sound card consumed some samples.
        fn play(&mut self, count: usize) {
            let take = count.min(self.queued.len());
            let drained: Vec<f32> = self.queued.drain(..take).collect();
            self.played.extend(drained);
        }

        fn finish(&mut self) {
            let rest = std::mem::take(&mut self.queued);
            self.played.extend(rest);
        }
    }

    impl SampleSink for FakeSink {
        fn write_slots(&self) -> usize {
            self.capacity - self.queued.len()
        }

        fn push_samples(&mut self, samples: &[f32]) -> usize {
            let room = self.write_slots();
            let take = room.min(samples.len());
            self.queued.extend_from_slice(&samples[..take]);
            take
        }
    }

    fn packets(count: usize, packet_len: usize) -> Vec<Vec<f32>> {
        let mut next = 0.0f32;
        (0..count)
            .map(|_| {
                (0..packet_len)
                    .map(|_| {
                        next += 1.0;
                        next
                    })
                    .collect()
            })
            .collect()
    }

    #[test]
    fn keeps_every_sample_when_the_sink_is_small() {
        let source = packets(40, 4096);
        let expected: Vec<f32> = source.iter().flatten().copied().collect();

        // Sink far smaller than the audio we push through it.
        let mut sink = FakeSink::new(8192);
        let mut feeder = Feeder::new();
        let mut queue = source.into_iter();

        loop {
            let mut decode = || match queue.next() {
                Some(samples) => DecodeStep::Samples(samples),
                None => DecodeStep::End,
            };

            let outcome = feeder.pump(&mut sink, 1024, 64, &mut decode);
            sink.play(3000);

            if outcome == PumpOutcome::Finished {
                break;
            }
        }

        sink.finish();
        assert_eq!(
            sink.played.len(),
            expected.len(),
            "samples were lost: got {} of {}",
            sink.played.len(),
            expected.len()
        );
        assert_eq!(sink.played, expected, "sample order or content changed");
    }

    #[test]
    fn leftover_is_kept_when_sink_is_full() {
        let mut sink = FakeSink::new(10);
        let mut feeder = Feeder::new();
        let mut done = false;

        let mut decode = move || {
            if done {
                DecodeStep::End
            } else {
                done = true;
                DecodeStep::Samples(vec![1.0; 25])
            }
        };

        let outcome = feeder.pump(&mut sink, 1, 8, &mut decode);
        assert_eq!(outcome, PumpOutcome::SinkFull);
        assert_eq!(sink.queued.len(), 10);
        assert_eq!(feeder.pending_samples(), 15, "leftover must be preserved");
    }

    #[test]
    fn reports_decoder_failure() {
        let mut sink = FakeSink::new(1024);
        let mut feeder = Feeder::new();
        let mut decode = || DecodeStep::Failed("broken".into());

        let outcome = feeder.pump(&mut sink, 1, 8, &mut decode);
        assert_eq!(outcome, PumpOutcome::Failed("broken".into()));
    }

    /// Documents the old behaviour that caused fast-forward playback:
    /// dropping the tail of a packet when the sink is full loses audio.
    #[test]
    fn dropping_the_tail_loses_audio() {
        let source = packets(10, 4096);
        let total: usize = source.iter().map(|p| p.len()).sum();

        let mut sink = FakeSink::new(8192);
        let mut accepted = 0usize;

        for packet in source {
            let written = sink.push_samples(&packet);
            accepted += written;
            sink.play(1000);
        }

        assert!(
            accepted < total,
            "expected the naive loop to lose samples, accepted {accepted} of {total}"
        );
    }
}
