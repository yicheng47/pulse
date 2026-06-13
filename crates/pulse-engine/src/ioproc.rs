//! Raw HAL IOProc lifecycle: `AudioDeviceCreateIOProcID` / `AudioDeviceStart`
//! / `AudioDeviceStop`. Never AUHAL — an implicit AudioConverter in the path
//! breaks the bit-perfect guarantee.
//!
//! The callback runs on the realtime audio thread: no allocation, no locks,
//! no syscalls. It only pulls from the rtrb consumer and writes into the
//! device buffer.

use std::{
    ffi::c_void,
    ptr::{self, NonNull},
    slice,
    sync::atomic::{AtomicU64, Ordering},
};

use objc2_core_audio::{
    AudioDeviceCreateIOProcID, AudioDeviceDestroyIOProcID, AudioDeviceIOProcID, AudioDeviceStart,
    AudioDeviceStop, AudioObjectID,
};
use objc2_core_audio_types::{AudioBuffer, AudioBufferList, AudioTimeStamp};
use rtrb::Consumer;

use crate::{EngineError, hal};

pub(crate) struct IoProc {
    device_id: AudioObjectID,
    proc_id: AudioDeviceIOProcID,
    state: Option<Box<CallbackState>>,
    running: bool,
}

impl IoProc {
    pub(crate) fn start(
        device_id: AudioObjectID,
        consumer: Consumer<u8>,
        bytes_per_frame: usize,
    ) -> Result<Self, EngineError> {
        let mut state = Box::new(CallbackState {
            consumer,
            bytes_per_frame,
            position_frames: AtomicU64::new(0),
            underrun_bytes: AtomicU64::new(0),
        });
        let mut proc_id = None;
        let status = unsafe {
            AudioDeviceCreateIOProcID(
                device_id,
                Some(render),
                state.as_mut() as *mut CallbackState as *mut c_void,
                NonNull::from(&mut proc_id),
            )
        };
        hal::check_status("AudioDeviceCreateIOProcID", status)?;

        let status = unsafe { AudioDeviceStart(device_id, proc_id) };
        if let Err(err) = hal::check_status("AudioDeviceStart", status) {
            let _ = unsafe { AudioDeviceDestroyIOProcID(device_id, proc_id) };
            return Err(err);
        }

        Ok(Self {
            device_id,
            proc_id,
            state: Some(state),
            running: true,
        })
    }

    pub(crate) fn position_frames(&self) -> u64 {
        self.state
            .as_ref()
            .map_or(0, |state| state.position_frames.load(Ordering::Relaxed))
    }
}

impl Drop for IoProc {
    fn drop(&mut self) {
        if self.running {
            let _ = unsafe { AudioDeviceStop(self.device_id, self.proc_id) };
            self.running = false;
        }
        let _ = unsafe { AudioDeviceDestroyIOProcID(self.device_id, self.proc_id) };
        self.state = None;
    }
}

struct CallbackState {
    consumer: Consumer<u8>,
    bytes_per_frame: usize,
    position_frames: AtomicU64,
    underrun_bytes: AtomicU64,
}

impl CallbackState {
    fn render(&mut self, output: NonNull<AudioBufferList>) {
        let buffers = audio_buffers_mut(output);
        for buffer in buffers {
            if buffer.mData.is_null() || buffer.mDataByteSize == 0 {
                continue;
            }

            let output = unsafe {
                slice::from_raw_parts_mut(buffer.mData.cast::<u8>(), buffer.mDataByteSize as usize)
            };
            let filled_len = {
                let (filled, remainder) = self.consumer.pop_partial_slice(output);
                let filled_len = filled.len();
                if !remainder.is_empty() {
                    remainder.fill(0);
                    self.underrun_bytes
                        .fetch_add(remainder.len() as u64, Ordering::Relaxed);
                }
                filled_len
            };

            if self.bytes_per_frame > 0 {
                self.position_frames.fetch_add(
                    (filled_len / self.bytes_per_frame) as u64,
                    Ordering::Relaxed,
                );
            }
        }
    }
}

unsafe extern "C-unwind" fn render(
    _device_id: AudioObjectID,
    _now: NonNull<AudioTimeStamp>,
    _input_data: NonNull<AudioBufferList>,
    _input_time: NonNull<AudioTimeStamp>,
    output_data: NonNull<AudioBufferList>,
    _output_time: NonNull<AudioTimeStamp>,
    client_data: *mut c_void,
) -> i32 {
    let Some(state) = (unsafe { client_data.cast::<CallbackState>().as_mut() }) else {
        return 0;
    };
    state.render(output_data);
    0
}

fn audio_buffers_mut<'a>(list: NonNull<AudioBufferList>) -> &'a mut [AudioBuffer] {
    let list = list.as_ptr();
    let buffer_count = unsafe { (*list).mNumberBuffers as usize };
    let buffers = unsafe { ptr::addr_of_mut!((*list).mBuffers).cast::<AudioBuffer>() };
    unsafe { slice::from_raw_parts_mut(buffers, buffer_count) }
}
