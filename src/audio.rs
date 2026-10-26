use std::sync::{Arc, Mutex};
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

const RATE: u32 = 24_000;
const SILENCE_PEAK: f32 = 0.008;
const SILENCE_RMS: f32 = 0.003;

pub fn input_devices() -> Vec<String> {
    let Ok(devices) = cpal::default_host().input_devices() else {
        return Vec::new();
    };
    let mut names: Vec<String> = devices
        .filter_map(|d| d.description().ok().map(|desc| desc.name().to_owned()))
        .collect();
    names.sort();
    names.dedup();
    names
}

fn device_label(device: &cpal::Device) -> String {
    device
        .description()
        .map(|d| d.name().to_owned())
        .unwrap_or_else(|_| device.to_string())
}

fn find_device(mic: &str) -> Result<cpal::Device, String> {
    if mic.is_empty() {
        return cpal::default_host()
            .default_input_device()
            .ok_or("no mic".into());
    }
    cpal::default_host()
        .input_devices()
        .map_err(|e| e.to_string())?
        .find(|d| d.description().is_ok_and(|desc| desc.name() == mic) || d.to_string() == mic)
        .ok_or(format!("mic `{mic}` not found"))
}

pub fn record_wav(stop: std::sync::mpsc::Receiver<()>, mic: &str) -> Result<Vec<u8>, String> {
    let device = find_device(mic)?;
    let label = device_label(&device);
    let supported = device.default_input_config().map_err(|e| e.to_string())?;
    let cfg = supported.config();
    let (pcm, channels, rate) = match supported.sample_format() {
        cpal::SampleFormat::F32 => capture::<f32>(&device, &cfg, stop)?,
        cpal::SampleFormat::I16 => capture::<i16>(&device, &cfg, stop)?,
        cpal::SampleFormat::U16 => capture::<u16>(&device, &cfg, stop)?,
        f => return Err(format!("unsupported mic format {f:?}")),
    };
    if pcm.is_empty() {
        return Err(format!(
            "mic `{label}` captured no frames. Please check it is plugged in and not in use"
        ));
    }
    let mono: Vec<f32> = pcm
        .chunks(channels.max(1) as usize)
        .map(|ch| ch.iter().sum::<f32>() / ch.len() as f32)
        .collect();
    let resampled = resample(&mono, rate);
    let seconds = resampled.len() as f32 / RATE as f32;
    let (peak, rms) = analyze(&resampled);
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut cursor = std::io::Cursor::new(Vec::new());
    {
        let mut out = hound::WavWriter::new(&mut cursor, spec).map_err(|e| e.to_string())?;
        for s in &resampled {
            out.write_sample((s.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16)
                .map_err(|e| e.to_string())?;
        }
        out.finalize().map_err(|e| e.to_string())?;
    }
    let wav = cursor.into_inner();
    save_debug_wav(&wav);
    if peak < SILENCE_PEAK && rms < SILENCE_RMS {
        return Err(format!(
            "mic `{label}` heard only silence ({seconds:.1}s, peak {peak:.3}). Please check input level, privacy settings, and device"
        ));
    }
    Ok(wav)
}

fn analyze(mono: &[f32]) -> (f32, f32) {
    let mut peak = 0.0f32;
    let mut sum_sq = 0.0f64;
    for &s in mono {
        let a = s.abs();
        if a > peak {
            peak = a;
        }
        sum_sq += (s as f64) * (s as f64);
    }
    let rms = if mono.is_empty() {
        0.0
    } else {
        (sum_sq / mono.len() as f64).sqrt() as f32
    };
    (peak, rms)
}

fn save_debug_wav(wav: &[u8]) {
    let path = std::env::temp_dir().join("trigger-last.wav");
    let _ = std::fs::write(path, wav);
}

fn capture<T>(
    device: &cpal::Device,
    cfg: &cpal::StreamConfig,
    stop: std::sync::mpsc::Receiver<()>,
) -> Result<(Vec<f32>, u16, u32), String>
where
    T: cpal::Sample<Float = f32> + cpal::SizedSample + Send + 'static,
{
    let buf = Arc::new(Mutex::new(Vec::<f32>::new()));
    let stream = device
        .build_input_stream::<T, _, _>(
            *cfg,
            {
                let buf = buf.clone();
                move |data: &[T], _| {
                    buf.lock()
                        .unwrap()
                        .extend(data.iter().map(|s| s.to_float_sample()));
                }
            },
            |err| crate::log_event(&format!("mic error: {err}")),
            None,
        )
        .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    let _ = stop.recv_timeout(Duration::from_secs(120));
    drop(stream);
    let pcm = Arc::try_unwrap(buf)
        .map_err(|_| "mic busy")?
        .into_inner()
        .map_err(|e| e.to_string())?;
    Ok((pcm, cfg.channels, cfg.sample_rate))
}

fn resample(mono: &[f32], from: u32) -> Vec<f32> {
    if from == RATE || mono.is_empty() {
        return mono.to_vec();
    }
    let len = (mono.len() as u64 * u64::from(RATE) / u64::from(from)) as usize;
    (0..len)
        .map(|i| {
            let p = i as f32 * from as f32 / RATE as f32;
            let j = p as usize;
            let f = p - j as f32;
            let a = mono[j.min(mono.len() - 1)];
            let b = mono[(j + 1).min(mono.len() - 1)];
            a + (b - a) * f
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silence_scores_below_speech() {
        let (peak, rms) = analyze(&[0.001; 2400]);
        assert!(peak < SILENCE_PEAK && rms < SILENCE_RMS);
        let loud: Vec<f32> = (0..2400).map(|i| (i as f32 * 0.01).sin() * 0.2).collect();
        let (peak, rms) = analyze(&loud);
        assert!(peak >= SILENCE_PEAK || rms >= SILENCE_RMS);
    }

    #[test]
    fn resample_halves_48k() {
        let mono: Vec<f32> = (0..480).map(|i| i as f32).collect();
        let out = resample(&mono, 48_000);
        assert_eq!(out.len(), 240);
        assert_eq!(out[0], 0.0);
        assert_eq!(out[1], 2.0);
    }
}
