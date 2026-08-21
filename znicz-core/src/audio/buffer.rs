use rtrb::{Consumer, Producer, RingBuffer};

/// About two seconds of stereo 48 kHz audio.
/// Small enough to stay in cache, large enough to survive short decode stalls.
pub const RING_BUFFER_SAMPLES: usize = 48_000 * 2 * 2;

pub fn new_pair() -> (Producer<f32>, Consumer<f32>) {
    RingBuffer::new(RING_BUFFER_SAMPLES)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn producer_and_consumer_are_independent() {
        let (mut producer, mut consumer) = new_pair();
        producer.push(0.5).unwrap();
        producer.push(-0.25).unwrap();
        assert_eq!(consumer.pop().unwrap(), 0.5);
        assert_eq!(consumer.pop().unwrap(), -0.25);
        assert!(consumer.pop().is_err());
    }
}
