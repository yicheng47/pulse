use std::{
    ffi::c_void,
    ptr::{self, NonNull},
    slice,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use objc2_core_audio::{
    AudioDeviceCreateIOProcID, AudioDeviceDestroyIOProcID, AudioDeviceIOProcID, AudioDeviceStart,
    AudioDeviceStop, AudioObjectID,
};
use objc2_core_audio_types::{
    AudioBuffer, AudioBufferList, AudioStreamBasicDescription, AudioTimeStamp,
};
use rtrb::Consumer;

use crate::{EngineError, hal};

struct CallbackContext {
    consumer: Consumer<u8>,
    position_frames: Arc<AtomicU64>,
    underrun_bytes: Arc<AtomicU64>,
    bytes_per_frame: usize,
}

pub(crate) struct RawSink {
    device_id: AudioObjectID,
    io_proc_id: AudioDeviceIOProcID,
    context: *mut CallbackContext,
    position_frames: Arc<AtomicU64>,
    underrun_bytes: Arc<AtomicU64>,
    bytes_per_frame: usize,
    running: bool,
}

// SAFETY: IntegerDeviceResources serializes every mutable RawSink access across the worker and
// shutdown threads. AudioDeviceStop does not return while the IOProc is executing, so Drop can
// destroy the stopped IOProc and free its callback context without aliasing the callback's access.
unsafe impl Send for RawSink {}

impl RawSink {
    pub(crate) fn start(
        device_id: AudioObjectID,
        consumer: Consumer<u8>,
        device_format: AudioStreamBasicDescription,
    ) -> Result<Self, EngineError> {
        let bytes_per_frame = usize::try_from(device_format.mBytesPerFrame).map_err(|_| {
            EngineError::UnsupportedFormat("device bytes per frame do not fit usize".to_string())
        })?;
        if bytes_per_frame == 0 {
            return Err(EngineError::UnsupportedFormat(
                "zero-byte device frames are not supported".to_string(),
            ));
        }

        let position_frames = Arc::new(AtomicU64::new(0));
        let underrun_bytes = Arc::new(AtomicU64::new(0));
        let context = Box::into_raw(Box::new(CallbackContext {
            consumer,
            position_frames: Arc::clone(&position_frames),
            underrun_bytes: Arc::clone(&underrun_bytes),
            bytes_per_frame,
        }));
        let mut io_proc_id = None;
        let status = unsafe {
            AudioDeviceCreateIOProcID(
                device_id,
                Some(output_io_proc),
                context.cast::<c_void>(),
                NonNull::from(&mut io_proc_id),
            )
        };
        if let Err(error) = hal::check_status("AudioDeviceCreateIOProcID", status) {
            unsafe {
                drop(Box::from_raw(context));
            }
            return Err(error);
        }
        let Some(io_proc) = io_proc_id else {
            unsafe {
                drop(Box::from_raw(context));
            }
            return Err(EngineError::UnsupportedFormat(
                "Core Audio returned no IOProc ID".to_string(),
            ));
        };

        let status = unsafe { AudioDeviceStart(device_id, Some(io_proc)) };
        if let Err(error) = hal::check_status("AudioDeviceStart", status) {
            let destroy_status = unsafe { AudioDeviceDestroyIOProcID(device_id, Some(io_proc)) };
            if hal::check_status("AudioDeviceDestroyIOProcID", destroy_status).is_ok() {
                unsafe {
                    drop(Box::from_raw(context));
                }
            }
            return Err(error);
        }

        Ok(Self {
            device_id,
            io_proc_id: Some(io_proc),
            context,
            position_frames,
            underrun_bytes,
            bytes_per_frame,
            running: true,
        })
    }

    pub(crate) fn restart(&mut self) -> Result<(), EngineError> {
        if self.running {
            return Ok(());
        }
        self.position_frames.store(0, Ordering::Relaxed);
        self.underrun_bytes.store(0, Ordering::Relaxed);
        let status = unsafe { AudioDeviceStart(self.device_id, self.io_proc_id) };
        hal::check_status("AudioDeviceStart", status)?;
        self.running = true;
        Ok(())
    }

    pub(crate) fn position_frames(&self) -> u64 {
        self.position_frames.load(Ordering::Relaxed)
    }

    pub(crate) fn underrun_frames(&self) -> u64 {
        self.underrun_bytes.load(Ordering::Relaxed) / self.bytes_per_frame as u64
    }

    pub(crate) fn stop(&mut self) -> Result<(), EngineError> {
        if self.running {
            let status = unsafe { AudioDeviceStop(self.device_id, self.io_proc_id) };
            hal::check_status("AudioDeviceStop", status)?;
            self.running = false;
        }

        let context = unsafe { &mut *self.context };
        let queued = context.consumer.slots();
        if queued > 0 {
            context
                .consumer
                .read_chunk(queued)
                .expect("reported consumer slots must be readable")
                .commit_all();
        }
        Ok(())
    }
}

impl Drop for RawSink {
    fn drop(&mut self) {
        let _ = self.stop();
        let status = unsafe { AudioDeviceDestroyIOProcID(self.device_id, self.io_proc_id) };
        if hal::check_status("AudioDeviceDestroyIOProcID", status).is_ok() {
            unsafe {
                drop(Box::from_raw(self.context));
            }
        }
    }
}

unsafe extern "C-unwind" fn output_io_proc(
    _device_id: AudioObjectID,
    _now: NonNull<AudioTimeStamp>,
    _input_data: NonNull<AudioBufferList>,
    _input_time: NonNull<AudioTimeStamp>,
    output_data: NonNull<AudioBufferList>,
    _output_time: NonNull<AudioTimeStamp>,
    client_data: *mut c_void,
) -> i32 {
    let context = unsafe { &mut *client_data.cast::<CallbackContext>() };
    let output = output_data.as_ptr();
    let buffer_count = unsafe { (*output).mNumberBuffers as usize };
    let buffers = unsafe { ptr::addr_of_mut!((*output).mBuffers).cast::<AudioBuffer>() };
    let mut filled_bytes = 0_usize;
    let mut underrun_bytes = 0_usize;

    for index in 0..buffer_count {
        let buffer = unsafe { &mut *buffers.add(index) };
        let len = buffer.mDataByteSize as usize;
        let Some(data) = NonNull::new(buffer.mData.cast::<u8>()) else {
            underrun_bytes = underrun_bytes.saturating_add(len);
            continue;
        };
        let output = unsafe { slice::from_raw_parts_mut(data.as_ptr(), len) };
        let (filled, remainder) = context.consumer.pop_partial_slice(output);
        filled_bytes = filled_bytes.saturating_add(filled.len());
        if !remainder.is_empty() {
            remainder.fill(0);
            underrun_bytes = underrun_bytes.saturating_add(remainder.len());
        }
    }

    context.position_frames.fetch_add(
        (filled_bytes / context.bytes_per_frame) as u64,
        Ordering::Relaxed,
    );
    context
        .underrun_bytes
        .fetch_add(underrun_bytes as u64, Ordering::Relaxed);
    0
}

#[cfg(test)]
mod tests {
    use std::mem;

    use objc2_core_audio_types::{AudioBuffer, AudioBufferList, AudioTimeStamp};
    use rtrb::RingBuffer;

    use super::*;

    #[test]
    fn io_proc_copies_ring_bytes_zero_fills_and_counts_frames() {
        let (mut producer, consumer) = RingBuffer::<u8>::new(8);
        let (pushed, _) = producer.push_partial_slice(&[1, 2, 3, 4, 5, 6]);
        assert_eq!(pushed.len(), 6);
        let position_frames = Arc::new(AtomicU64::new(0));
        let underrun_bytes = Arc::new(AtomicU64::new(0));
        let mut context = CallbackContext {
            consumer,
            position_frames: Arc::clone(&position_frames),
            underrun_bytes: Arc::clone(&underrun_bytes),
            bytes_per_frame: 2,
        };
        let mut output = [0xff_u8; 8];
        let mut output_list = AudioBufferList {
            mNumberBuffers: 1,
            mBuffers: [AudioBuffer {
                mNumberChannels: 2,
                mDataByteSize: output.len() as u32,
                mData: output.as_mut_ptr().cast::<c_void>(),
            }],
        };
        let mut empty_list = AudioBufferList {
            mNumberBuffers: 0,
            mBuffers: [AudioBuffer {
                mNumberChannels: 0,
                mDataByteSize: 0,
                mData: ptr::null_mut(),
            }],
        };
        let mut timestamp: AudioTimeStamp = unsafe { mem::zeroed() };

        unsafe {
            output_io_proc(
                7,
                NonNull::from(&mut timestamp),
                NonNull::from(&mut empty_list),
                NonNull::from(&mut timestamp),
                NonNull::from(&mut output_list),
                NonNull::from(&mut timestamp),
                (&mut context as *mut CallbackContext).cast::<c_void>(),
            );
        }

        assert_eq!(output, [1, 2, 3, 4, 5, 6, 0, 0]);
        assert_eq!(position_frames.load(Ordering::Relaxed), 3);
        assert_eq!(underrun_bytes.load(Ordering::Relaxed), 2);
    }
}
