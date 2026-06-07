//! Raw HAL IOProc lifecycle: `AudioDeviceCreateIOProcID` / `AudioDeviceStart`
//! / `AudioDeviceStop`. Never AUHAL — an implicit AudioConverter in the path
//! breaks the bit-perfect guarantee.
//!
//! The callback runs on the realtime audio thread: no allocation, no locks,
//! no syscalls. It only pulls from the rtrb consumer and writes into the
//! device buffer.
