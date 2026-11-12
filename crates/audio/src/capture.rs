//! Native microphone capture via CPAL into a bounded lock-free ring.
//!
//! Real-time rules (docs/03 §3): the CPAL callback only converts samples and
//! pushes them into a preallocated SPSC ring. It never locks, allocates, logs or
//! emits events. Faults are reported through atomics and turn into an abort on
//! the consumer side; a truncated recording is never silently transcribed.
//!
//! The stream is always opened on a *specific* device id, never "the default
//! device", because CPAL reroutes default-device streams on route changes
//! (`ErrorKind::DeviceChanged`), which would be a silent microphone
//! substitution (docs/11 invariants).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{DeviceId, ErrorKind, SampleFormat, StreamConfig};

use crate::convert::{i16_to_f32, u16_to_f32};

/// Seconds of native-rate audio the ring holds before declaring overflow.
const RING_SECONDS: usize = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputDeviceInfo {
    /// Opaque, session-stable id (CPAL `DeviceId` string form).
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeFormat {
    pub sample_rate: u32,
    pub channels: u16,
    pub sample_format: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CaptureError {
    #[error("no input device available")]
    NoDevice,
    #[error("selected input device is not available")]
    DeviceNotAvailable,
    #[error("microphone permission denied")]
    PermissionDenied,
    #[error("unsupported native sample format")]
    UnsupportedFormat,
    #[error("audio host error")]
    Host,
    #[error("capture ring overflowed; recording aborted")]
    Overflow,
    #[error("microphone disconnected or rerouted; recording aborted")]
    Disconnected,
    #[error("capture thread failed")]
    Thread,
}

const FAULT_NONE: u8 = 0;
const FAULT_DISCONNECTED: u8 = 1;
const FAULT_XRUN: u8 = 2;
const FAULT_OTHER: u8 = 3;

pub fn list_input_devices() -> Result<Vec<InputDeviceInfo>, CaptureError> {
    let host = cpal::default_host();
    let default_id = host
        .default_input_device()
        .and_then(|d| d.id().ok())
        .map(|id| id.to_string());
    let devices = host.input_devices().map_err(map_err)?;
    Ok(devices
        .filter_map(|d| {
            let id = d.id().ok()?.to_string();
            let name = d
                .description()
                .map(|desc| desc.name().to_owned())
                .unwrap_or_else(|_| id.clone());
            Some(InputDeviceInfo {
                is_default: default_id.as_deref() == Some(id.as_str()),
                id,
                name,
            })
        })
        .collect())
}

/// Consumer side; owned by the DSP thread.
pub struct CaptureReader {
    consumer: rtrb::Consumer<f32>,
    overflow: Arc<AtomicBool>,
    fault: Arc<AtomicU8>,
    pub format: NativeFormat,
}

impl CaptureReader {
    /// Drains available interleaved native-rate samples into `out`.
    pub fn drain_into(&mut self, out: &mut Vec<f32>) -> Result<usize, CaptureError> {
        match self.fault.load(Ordering::Acquire) {
            FAULT_NONE => {}
            FAULT_XRUN => return Err(CaptureError::Overflow),
            FAULT_DISCONNECTED => return Err(CaptureError::Disconnected),
            _ => return Err(CaptureError::Host),
        }
        if self.overflow.load(Ordering::Acquire) {
            return Err(CaptureError::Overflow);
        }
        let n = self.consumer.slots();
        if n == 0 {
            return Ok(0);
        }
        let chunk = self
            .consumer
            .read_chunk(n)
            .map_err(|_| CaptureError::Host)?;
        let (a, b) = chunk.as_slices();
        out.extend_from_slice(a);
        out.extend_from_slice(b);
        chunk.commit_all();
        Ok(n)
    }
}

/// Owns the stream on a dedicated thread (CPAL streams are not `Send` on every
/// platform). Dropping or calling [`CaptureHandle::stop`] closes the microphone.
pub struct CaptureHandle {
    stop_tx: Option<mpsc::Sender<()>>,
    join: Option<JoinHandle<()>>,
}

impl CaptureHandle {
    pub fn stop(&mut self) {
        if let Some(tx) = self.stop_tx.take() {
            let _ = tx.send(());
        }
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

impl Drop for CaptureHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Opens capture on `device_id`, or on the device that is currently the
/// system default (resolved to its concrete id first).
pub fn start(device_id: Option<&str>) -> Result<(CaptureHandle, CaptureReader), CaptureError> {
    let requested = device_id.map(str::to_owned);
    let (ready_tx, ready_rx) = mpsc::channel::<Result<CaptureReader, CaptureError>>();
    let (stop_tx, stop_rx) = mpsc::channel::<()>();

    let join = thread::Builder::new()
        .name("audio-capture".into())
        .spawn(move || {
            let stream = match open_stream(requested.as_deref()) {
                Ok((stream, reader)) => {
                    let _ = ready_tx.send(Ok(reader));
                    stream
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                    return;
                }
            };
            // Hold the stream until asked to stop or the handle is dropped.
            let _ = stop_rx.recv();
            let _ = stream.pause();
            drop(stream);
        })
        .map_err(|_| CaptureError::Thread)?;

    let reader = ready_rx
        .recv_timeout(Duration::from_secs(5))
        .map_err(|_| CaptureError::Thread)??;
    Ok((
        CaptureHandle {
            stop_tx: Some(stop_tx),
            join: Some(join),
        },
        reader,
    ))
}

fn open_stream(device_id: Option<&str>) -> Result<(cpal::Stream, CaptureReader), CaptureError> {
    let host = cpal::default_host();
    let id: DeviceId = match device_id {
        Some(s) => s.parse().map_err(|_| CaptureError::DeviceNotAvailable)?,
        None => host
            .default_input_device()
            .ok_or(CaptureError::NoDevice)?
            .id()
            .map_err(map_err)?,
    };
    let device = host
        .device_by_id(&id)
        .ok_or(CaptureError::DeviceNotAvailable)?;
    let supported = device.default_input_config().map_err(map_err)?;
    let format = NativeFormat {
        sample_rate: supported.sample_rate(),
        channels: supported.channels(),
        sample_format: match supported.sample_format() {
            SampleFormat::F32 => "f32",
            SampleFormat::I16 => "i16",
            SampleFormat::U16 => "u16",
            _ => return Err(CaptureError::UnsupportedFormat),
        },
    };
    if format.channels == 0 || !(8_000..=384_000).contains(&format.sample_rate) {
        return Err(CaptureError::UnsupportedFormat);
    }
    let sample_format = supported.sample_format();
    let config: StreamConfig = supported.into();

    let cap = format.sample_rate as usize * format.channels as usize * RING_SECONDS;
    let (mut producer, consumer) = rtrb::RingBuffer::<f32>::new(cap);
    let overflow = Arc::new(AtomicBool::new(false));
    let fault = Arc::new(AtomicU8::new(FAULT_NONE));

    let fault_cb = Arc::clone(&fault);
    let err_fn = move |err: cpal::Error| {
        let code = match err.kind() {
            ErrorKind::DeviceNotAvailable | ErrorKind::DeviceChanged => FAULT_DISCONNECTED,
            ErrorKind::Xrun => FAULT_XRUN,
            _ => FAULT_OTHER,
        };
        let _ = fault_cb.compare_exchange(FAULT_NONE, code, Ordering::AcqRel, Ordering::Acquire);
    };

    macro_rules! build {
        ($t:ty, $conv:expr) => {{
            let ovf = Arc::clone(&overflow);
            device.build_input_stream::<$t, _, _>(
                config,
                move |data: &[$t], _| {
                    if ovf.load(Ordering::Relaxed) {
                        return;
                    }
                    for &s in data {
                        if producer.push($conv(s)).is_err() {
                            ovf.store(true, Ordering::Release);
                            return;
                        }
                    }
                },
                err_fn,
                None,
            )
        }};
    }

    let stream = match sample_format {
        SampleFormat::F32 => build!(f32, |s: f32| if s.is_finite() {
            s.clamp(-1.0, 1.0)
        } else {
            f32::NAN
        }),
        SampleFormat::I16 => build!(i16, i16_to_f32),
        SampleFormat::U16 => build!(u16, u16_to_f32),
        _ => return Err(CaptureError::UnsupportedFormat),
    }
    .map_err(map_err)?;
    stream.play().map_err(map_err)?;
    Ok((
        stream,
        CaptureReader {
            consumer,
            overflow,
            fault,
            format,
        },
    ))
}

fn map_err(e: cpal::Error) -> CaptureError {
    match e.kind() {
        ErrorKind::DeviceNotAvailable => CaptureError::DeviceNotAvailable,
        ErrorKind::PermissionDenied => CaptureError::PermissionDenied,
        ErrorKind::UnsupportedConfig => CaptureError::UnsupportedFormat,
        _ => CaptureError::Host,
    }
}
