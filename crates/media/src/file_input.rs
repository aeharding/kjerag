//! Independent demux cursors over one bounded, shared file-byte cache.
//!
//! Audio still has its own demuxer: camera interleave gaps must not starve it.
//! But two network file handles contend even when video-only reading is fast.
//! This layer coalesces their small reads and retains no decoded/GPU resources.

use std::collections::VecDeque;
use std::ffi::{CString, c_int, c_void};
use std::fs::File;
use std::io;
use std::ops::{Deref, DerefMut};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::{Arc, Mutex};

use ffmpeg_next as ff;

use crate::Fallible;

const BLOCK: usize = 1024 * 1024;
const BLOCKS: usize = 16;
const AVIO_BUFFER: usize = 32 * 1024;

pub(crate) struct SharedFile {
    path: PathBuf,
    file: File,
    size: u64,
    // MRU at the back. The lock serializes file reads, never decoded delivery,
    // renderer work, or an audio-device callback.
    blocks: Mutex<VecDeque<(u64, Vec<u8>)>>,
}

impl SharedFile {
    fn open(path: &Path) -> io::Result<Arc<Self>> {
        let file = File::open(path)?;
        let size = file.metadata()?.len();
        if size > i64::MAX as u64 {
            return Err(io::Error::other("input size exceeds the seek range"));
        }
        Ok(Arc::new(Self {
            path: path.to_owned(),
            file,
            size,
            blocks: Mutex::new(VecDeque::new()),
        }))
    }

    fn read_at(&self, target: &mut [u8], at: u64) -> io::Result<usize> {
        if target.is_empty() || at >= self.size {
            return Ok(0);
        }
        let start = at / BLOCK as u64 * BLOCK as u64;
        let within = (at - start) as usize;
        let mut blocks = self
            .blocks
            .lock()
            .map_err(|_| io::Error::other("input byte cache is poisoned"))?;
        let bytes = if let Some(index) = blocks.iter().position(|(offset, _)| *offset == start) {
            blocks
                .remove(index)
                .ok_or_else(|| io::Error::other("input cache entry disappeared"))?
                .1
        } else {
            if blocks.len() == BLOCKS {
                blocks.pop_front();
            }
            let length = (self.size - start).min(BLOCK as u64) as usize;
            let mut bytes = vec![0; length];
            let mut filled = 0;
            while filled < length {
                match self
                    .file
                    .read_at(&mut bytes[filled..], start + filled as u64)
                {
                    Ok(0) => break,
                    Ok(count) => filled += count,
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(error) => return Err(error),
                }
            }
            bytes.truncate(filled);
            bytes
        };
        let count = target.len().min(bytes.len().saturating_sub(within));
        if count != 0 {
            target[..count].copy_from_slice(&bytes[within..within + count]);
        }
        blocks.push_back((start, bytes));
        Ok(count)
    }
}

struct Cursor {
    source: Arc<SharedFile>,
    at: u64,
    error: Arc<Mutex<Option<String>>>,
}

impl Cursor {
    fn fail(&self, error: io::Error) -> c_int {
        *self.error.lock().unwrap_or_else(|e| e.into_inner()) = Some(error.to_string());
        -error.raw_os_error().unwrap_or(ff::error::EIO)
    }
}

unsafe extern "C" fn read(opaque: *mut c_void, target: *mut u8, length: c_int) -> c_int {
    // Only this cursor's libavformat owner invokes its callbacks, serially.
    let cursor = unsafe { &mut *opaque.cast::<Cursor>() };
    if length <= 0 {
        return -ff::error::EINVAL;
    }
    let target = unsafe { std::slice::from_raw_parts_mut(target, length as usize) };
    match cursor.source.read_at(target, cursor.at) {
        Ok(0) => ff::ffi::AVERROR_EOF,
        Ok(count) => {
            cursor.at += count as u64;
            count as c_int
        }
        Err(error) => cursor.fail(error),
    }
}

unsafe extern "C" fn seek(opaque: *mut c_void, offset: i64, whence: c_int) -> i64 {
    let cursor = unsafe { &mut *opaque.cast::<Cursor>() };
    if whence & ff::ffi::AVSEEK_SIZE != 0 {
        return cursor.source.size as i64;
    }
    // POSIX SEEK_SET/CUR/END, after removing FFmpeg's force-seek modifier.
    let base = match whence & !ff::ffi::AVSEEK_FORCE {
        0 => 0,
        1 => cursor.at,
        2 => cursor.source.size,
        _ => return i64::from(-ff::error::EINVAL),
    };
    let at = i128::from(base) + i128::from(offset);
    if !(0..=i128::from(i64::MAX)).contains(&at) {
        return i64::from(-ff::error::EINVAL);
    }
    cursor.at = at as u64;
    at as i64
}

struct Io {
    context: *mut ff::ffi::AVIOContext,
    cursor: *mut Cursor,
    error: Arc<Mutex<Option<String>>>,
    source: Arc<SharedFile>,
}

// Unique AVIO/cursor ownership travels with its single Send demuxer. Only the
// immutable file/cache is shared; no context or cursor is used concurrently.
unsafe impl Send for Io {}

impl Io {
    fn new(source: Arc<SharedFile>, at: u64) -> Fallible<Self> {
        let error = Arc::new(Mutex::new(None));
        let cursor = Box::into_raw(Box::new(Cursor {
            source: source.clone(),
            at,
            error: error.clone(),
        }));
        let buffer = unsafe { ff::ffi::av_malloc(AVIO_BUFFER) }.cast::<u8>();
        if buffer.is_null() {
            unsafe {
                drop(Box::from_raw(cursor));
            }
            return Err(ff::Error::Other {
                errno: ff::error::ENOMEM,
            }
            .into());
        }
        let context = unsafe {
            ff::ffi::avio_alloc_context(
                buffer,
                AVIO_BUFFER as c_int,
                0,
                cursor.cast(),
                Some(read),
                None,
                Some(seek),
            )
        };
        if context.is_null() {
            unsafe {
                ff::ffi::av_free(buffer.cast());
                drop(Box::from_raw(cursor));
            }
            return Err(ff::Error::Other {
                errno: ff::error::ENOMEM,
            }
            .into());
        }
        unsafe {
            (*context).seekable = ff::ffi::AVIO_SEEKABLE_NORMAL;
            (*context).pos = at as i64;
        }
        Ok(Self {
            context,
            cursor,
            error,
            source,
        })
    }

    fn failure(&self) -> Option<String> {
        self.error.lock().unwrap_or_else(|e| e.into_inner()).take()
    }
}

impl Drop for Io {
    fn drop(&mut self) {
        // libavformat may replace the original AVIO buffer. Free its current
        // buffer after closing the format, then the AVIO and opaque cursor.
        unsafe {
            ff::ffi::av_free((*self.context).buffer.cast());
            ff::ffi::avio_context_free(&mut self.context);
            drop(Box::from_raw(self.cursor));
        }
    }
}

pub(crate) struct Input {
    // Field drop order is intentional: format closes before its custom IO.
    format: ff::format::context::Input,
    io: Io,
}

impl Input {
    pub(crate) fn open(path: &Path) -> Fallible<Self> {
        // MOV retains per-stream AVIO pointers during header inspection.
        // Install custom IO before opening, never replace an opened input's pb.
        Self::open_shared(SharedFile::open(path)?)
    }

    pub(crate) fn source(&self) -> Arc<SharedFile> {
        self.io.source.clone()
    }

    pub(crate) fn open_shared(source: Arc<SharedFile>) -> Fallible<Self> {
        let path = CString::new(source.path.as_os_str().as_bytes())?;
        let io = Io::new(source, 0)?;
        let mut context = unsafe { ff::ffi::avformat_alloc_context() };
        if context.is_null() {
            return Err(ff::Error::Other {
                errno: ff::error::ENOMEM,
            }
            .into());
        }
        unsafe {
            (*context).pb = io.context;
            (*context).flags |= ff::ffi::AVFMT_FLAG_CUSTOM_IO;
        }
        let result = unsafe {
            ff::ffi::avformat_open_input(&mut context, path.as_ptr(), ptr::null(), ptr::null_mut())
        };
        if result < 0 {
            return Err(io
                .failure()
                .unwrap_or_else(|| ff::Error::from(result).to_string())
                .into());
        }
        let input = Self {
            format: unsafe { ff::format::context::Input::wrap(context) },
            io,
        };
        let result = unsafe { ff::ffi::avformat_find_stream_info(context, ptr::null_mut()) };
        if result < 0 {
            return Err(input
                .failure()
                .unwrap_or_else(|| ff::Error::from(result).to_string())
                .into());
        }
        Ok(input)
    }

    pub(crate) fn failure(&self) -> Option<String> {
        self.io.failure()
    }
}

impl Deref for Input {
    type Target = ff::format::context::Input;
    fn deref(&self) -> &Self::Target {
        &self.format
    }
}

impl DerefMut for Input {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.format
    }
}

#[cfg(test)]
mod tests;
