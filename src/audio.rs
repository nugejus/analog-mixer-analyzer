use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};
use rtrb::{Consumer, Producer, RingBuffer};
use std::{
    f32::consts::TAU,
    thread,
    time::{Duration, Instant},
};

pub struct Input {
    pub sample_rate: f32,
    pub channels: usize,
    pub rx: Consumer<f32>,
    _keep: Box<dyn std::any::Any>,
}

// Not `to_string()`: cpal's Display errors (and to_string panics) when a device vanishes mid-call.
fn name(d: &cpal::Device) -> Option<String> {
    d.description().ok().map(|x| x.name().to_owned())
}

pub fn device_names() -> Vec<String> {
    cpal::default_host()
        .input_devices()
        .map(|ds| ds.filter_map(|d| name(&d)).collect())
        .unwrap_or_default()
}

pub fn default_device_name() -> Option<String> {
    cpal::default_host()
        .default_input_device()
        .as_ref()
        .and_then(name)
}

/// Opens the input device named `device`, or the default one if that name is not found.
pub fn open(device: Option<&str>) -> Result<Input, String> {
    let host = cpal::default_host();
    let dev = device
        .and_then(|want| {
            host.input_devices()
                .ok()?
                .find(|d| name(d).as_deref() == Some(want))
        })
        .or_else(|| host.default_input_device())
        .ok_or(crate::lang::tr("No input device", "입력 장치가 없습니다"))?;
    let sup = dev.default_input_config().map_err(|e| e.to_string())?;
    let (sample_rate, channels) = (sup.sample_rate() as f32, sup.channels() as usize);
    let (tx, rx) = RingBuffer::new(sample_rate as usize * channels * 2);
    let cfg = sup.config();
    let stream = match sup.sample_format() {
        SampleFormat::F32 => build::<f32>(&dev, cfg, tx),
        SampleFormat::I16 => build::<i16>(&dev, cfg, tx),
        SampleFormat::I32 => build::<i32>(&dev, cfg, tx),
        SampleFormat::U16 => build::<u16>(&dev, cfg, tx),
        f => return Err(crate::tr!("Unsupported sample format: {f}", "지원하지 않는 샘플 형식입니다: {f}")),
    }
    .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    Ok(Input {
        sample_rate,
        channels,
        rx,
        _keep: Box::new(stream),
    })
}

fn build<T>(
    dev: &cpal::Device,
    cfg: cpal::StreamConfig,
    mut tx: Producer<f32>,
) -> Result<cpal::Stream, cpal::Error>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    dev.build_input_stream(
        cfg,
        // Whole buffer or nothing: dropping single samples would shift the channel interleave.
        move |data: &[T], _: &cpal::InputCallbackInfo| {
            if let Ok(chunk) = tx.write_chunk_uninit(data.len()) {
                chunk.fill_from_iter(data.iter().map(|&s| s.to_sample::<f32>()));
            }
        },
        |e| eprintln!("audio stream error: {e}"),
        None,
    )
}

const DEMO_SR: f32 = 48_000.0;

/// 48 kHz mono synthetic voice, fed every 10 ms by a thread that ends when `rx` is dropped.
pub fn open_demo() -> Input {
    let (mut tx, rx) = RingBuffer::new(DEMO_SR as usize * 2);
    let start = Instant::now();
    let keep = thread::spawn(move || {
        let mut n = 0;
        while !tx.is_abandoned() {
            let due = (start.elapsed().as_secs_f64() * DEMO_SR as f64) as u64;
            for i in n..due {
                let _ = tx.push(demo_voice(i));
            }
            n = due;
            thread::sleep(Duration::from_millis(10));
        }
    });
    Input {
        sample_rate: DEMO_SR,
        channels: 1,
        rx,
        _keep: Box::new(keep),
    }
}

/// T4 `voiced` (120 Hz, 5 Hz vibrato, 10 harmonics), gated 0.6 s on / 0.25 s off, plus a second
/// fundamental for a +6 dB boom at 120 Hz. Everything repeats every 17 s, so `t` stays small.
fn demo_voice(n: u64) -> f32 {
    let t = (n % (17 * DEMO_SR as u64)) as f32 / DEMO_SR;
    if t % 0.85 >= 0.6 {
        return 0.0;
    }
    let phase = TAU * 120.0 * (t - 0.05 / (TAU * 5.0) * (TAU * 5.0 * t).cos());
    let voiced: f32 = (1..=10)
        .map(|h| (0.1 / h as f32) * (h as f32 * phase).sin())
        .sum();
    voiced + 0.1 * phase.sin()
}

/// Channel `ch` of interleaved samples; channel 0 when `ch` does not exist.
pub fn pick_channel(interleaved: &[f32], channels: usize, ch: usize) -> Vec<f32> {
    let ch = if ch < channels { ch } else { 0 };
    interleaved
        .iter()
        .skip(ch)
        .step_by(channels.max(1))
        .copied()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pick_channel_falls_back_to_first() {
        assert_eq!(pick_channel(&[1.0, 2.0, 3.0, 4.0], 2, 1), vec![2.0, 4.0]);
        assert_eq!(pick_channel(&[1.0, 2.0, 3.0, 4.0], 2, 0), vec![1.0, 3.0]);
        assert_eq!(pick_channel(&[1.0, 2.0], 1, 1), vec![1.0, 2.0]);
    }
    #[test]
    fn demo_input_streams_samples() {
        let inp = open_demo();
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert_eq!((inp.sample_rate, inp.channels), (48_000.0, 1));
        assert!(inp.rx.slots() > 1000);
    }
}
