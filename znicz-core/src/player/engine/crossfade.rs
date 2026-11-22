//! Crossfade: the engine side. The mix itself lives in `audio::crossfade`;
//! this opens the incoming track, runs the crossfade in the pump, and hands
//! the incoming decoder over when it ends or is heard.

use std::time::Duration;

use crate::audio::convert::RateConverter;
use crate::audio::crossfade::{Crossfade, DecodedTrack, output_frames};
use crate::audio::feeder::PumpOutcome;
use crate::audio::source::AudioDecoder;
use crate::player::fade;
use crate::player::gapless::PendingTrack;

use super::{MAX_PACKETS_PER_PUMP, MIN_WRITE_SLOTS, PlayerEngine};

impl PlayerEngine {
    /// Pump a running crossfade in place of the main decoder.
    pub(super) fn pump_crossfade(&mut self, mut crossfade: Crossfade) {
        let output_channels = self.output_channels();
        let mut step = || crossfade.next_step(output_channels);
        let outcome = self.feeder.pump(
            &mut self.output,
            MIN_WRITE_SLOTS,
            MAX_PACKETS_PER_PUMP,
            &mut step,
        );

        match outcome {
            PumpOutcome::SinkFull => self.crossfade = Some(crossfade),
            // A crossfade step only yields once it is done; a `Finished` would
            // mean the same: the incoming track is what plays on.
            PumpOutcome::Yielded | PumpOutcome::Finished => self.hand_over(crossfade),
            PumpOutcome::Failed(message) => {
                self.emit_error(message);
                let _ = self.stop();
            }
        }
    }

    /// The playing track reached its crossfade point: open the next row and
    /// mix the two. When that is not possible the track plays on, and its end
    /// joins or drains as usual.
    pub(super) fn start_crossfade(
        &mut self,
        outgoing: AudioDecoder,
        converter: Option<RateConverter>,
    ) {
        let file_rate = outgoing.sample_rate();
        let length = fade::crossfade_length(
            self.fades.crossfade,
            self.track_duration,
            outgoing.position_frames(),
            file_rate,
        );
        let Some((track, incoming)) = self.open_crossfade_track(&outgoing, length) else {
            // No retry for this track: its end joins or drains as usual.
            self.crossfade_from = None;
            self.crossfade_declined = true;
            self.decoder = Some(outgoing);
            self.converter = converter;
            return;
        };

        let output_rate = self.output.sample_rate();
        let output_channels = self.output_channels();
        // Same format as the outgoing track (checked when it was opened), so
        // the incoming side needs a converter exactly when the file's rate
        // differs from the output's.
        let incoming_converter = (file_rate != output_rate)
            .then(|| RateConverter::new(file_rate, output_rate, output_channels));
        let duration = track.info.duration;

        self.pending.push(track);
        self.crossfade_from = None;
        self.crossfade = Some(Crossfade::new(
            DecodedTrack {
                decoder: outgoing,
                converter,
            },
            DecodedTrack {
                decoder: incoming,
                converter: incoming_converter,
            },
            duration,
            output_frames(length, file_rate, output_rate),
            output_channels,
        ));
    }

    /// The next row, opened, if a crossfade of `length` frames (in the file's
    /// rate) fits: a local file of the same format, long enough for it.
    fn open_crossfade_track(
        &mut self,
        outgoing: &AudioDecoder,
        length: u64,
    ) -> Option<(PendingTrack, AudioDecoder)> {
        if length == 0 {
            return None;
        }
        let (track, incoming) = self.open_next_row(outgoing)?;
        fade::incoming_long_enough(track.info.duration, length, outgoing.sample_rate())
            .then_some((track, incoming))
    }

    /// The crossfade is over: the incoming track becomes the current decoder.
    fn hand_over(&mut self, crossfade: Crossfade) {
        let (incoming, duration) = crossfade.into_incoming();
        self.decoder = Some(incoming.decoder);
        self.converter = incoming.converter;
        self.crossfade_declined = false;
        self.arm_crossfade(duration);
    }

    /// A crossfade whose incoming track is already playing ends now, so that
    /// a seek applies to that track. Before it is heard, the reopen in `seek`
    /// drops the crossfade instead.
    pub(super) fn end_heard_crossfade(&mut self) {
        if self.pending.is_empty() {
            self.end_running_crossfade();
        }
    }

    /// End a running crossfade now: the incoming track becomes current.
    pub(super) fn end_running_crossfade(&mut self) {
        if let Some(crossfade) = self.crossfade.take() {
            self.hand_over(crossfade);
        }
    }

    /// Work out where the current decoder's crossfade starts, if it has one.
    /// The decoder must already be set.
    pub(super) fn arm_crossfade(&mut self, duration: Option<Duration>) {
        self.track_duration = duration;
        self.crossfade_from = self.decoder.as_ref().and_then(|decoder| {
            fade::crossfade_from(
                self.fades.crossfade,
                duration,
                decoder.sample_rate(),
                decoder.is_stream(),
            )
        });
    }
}
