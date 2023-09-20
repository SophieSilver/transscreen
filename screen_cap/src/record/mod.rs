pub mod encoded_buffer;

use std::{io, sync::Arc, time::Instant};

use parking_lot::{Condvar, Mutex};
use scrap::Display;
use thiserror::Error;
use utils::{
    contiguous::WriteDataError,
    threading::{ThreadLoop, ThreadWork},
};
use x264::{Encoder, Image};

use crate::{capture::ThreadedCapturer, frame::FrameError, record::encoded_buffer::Metadata};

use self::encoded_buffer::{EncodedBuffer, EncodedBufferView, EncodedDataGuard};

struct RecordWorker {
    capturer: ThreadedCapturer,
    encoder: Encoder,
    width: i32,
    height: i32,
    // TODO: change the data structure
    data_buf: EncodedBuffer,
    timebase: f64,
    record_start_time: Instant,
    buffered_frames: usize,
}

impl RecordWorker {
    fn update(&mut self) -> Result<EncodeStatus, RecordError> {
        // get the frame
        let frame = match self.capturer.frame() {
            Ok(f) => f,
            // ignore skipped frames
            Err(e) => match e {
                FrameError::Skipped => {
                    return Ok(EncodeStatus::Skipped);
                }
                FrameError::Error(e) => return Err(e.into()),
            },
        };

        let frame_data = if cfg!(target_os = "macos") {
            // stride is different on macos
            // https://github.com/quadrupleslap/scrap/issues/44#issuecomment-1486345836
            let w = self.width as usize;
            let h = self.height as usize;

            &frame[..w * h * 4]
        } else {
            &frame
        };

        let image = Image::bgra(self.width, self.height, frame_data);

        // actually encoding
        let elapsed = self.record_start_time.elapsed().as_secs_f64();
        let timestamp = (elapsed * self.timebase) as i64;
        let (data, picture) = self.encoder.encode(timestamp, image)?;

        // update the buffer
        let metadata = Metadata {
            is_key: picture.keyframe(),
        };

        if self.buffered_frames == 0 {
            // write flush is a bit more efficient since it immediately writes to the shared ring buffer
            self.data_buf.write_flush(data.entirety(), metadata)?;
            
            Ok(EncodeStatus::Flushed)
        } else {
            // write into a local buffer
            self.data_buf.write(data.entirety(), metadata);
            // only copy data from the local buffer once its length reaches self.buffered_frames
            if self.buffered_frames < self.data_buf.write_buf_len() {
                self.data_buf.flush()?;

                Ok(EncodeStatus::Flushed)
            } else {
                Ok(EncodeStatus::PreBuffered)
            }
        }
    }
}

impl ThreadWork for RecordWorker {
    type WorkResult = Result<EncodeStatus, RecordError>;

    fn work(&mut self) -> Self::WorkResult {
        self.update()
    }
}

#[derive(Debug, Error)]
pub enum RecordError {
    #[error(transparent)]
    FrameError(#[from] io::Error),
    // x264::Error is zero sized and doesn't even implement the Error trait
    #[error("there has been an error while encoding a frame")]
    EncodeError,

    #[error(transparent)]
    WriteDataError(#[from] WriteDataError),
}

// can't do this with a macro because x264::Error doesn't implement the Error trait
impl From<x264::Error> for RecordError {
    fn from(_: x264::Error) -> Self {
        Self::EncodeError
    }
}

pub struct Recorder {
    thread_loop: ThreadLoop<RecordWorker>,
    data_buf: EncodedBufferView,
    headers: Box<[u8]>,
}

impl Recorder {
    pub fn new<F, G>(
        capturer_settings: CapturerSettings<F>,
        buffering_settings: BufferingSettings,
        encoder_settings: EncoderSettings<G>,
    ) -> Self
    where
        F: FnMut() -> Display + Send + 'static,
        G: FnOnce() -> Encoder + Send + 'static,
    {
        // destructuring arguments arguments
        let CapturerSettings {
            mut display_factory,
            target_rate,
        } = capturer_settings;

        let BufferingSettings {
            buffer_capacity,
            buffered_frames,
        } = buffering_settings;

        let EncoderSettings {
            encoder_factory,
            timebase,
        } = encoder_settings;

        let display = display_factory();

        let width = display.width() as i32;
        let height = display.height() as i32;

        let capturer = ThreadedCapturer::new(display_factory, target_rate);

        let data_buf = EncodedBuffer::new(buffer_capacity);
        let data_buf_view = data_buf.view();

        // getting the headers from the thread with the encoder
        let headers_dest: Arc<(Mutex<Option<Box<[u8]>>>, Condvar)> = Arc::default();
        let headers_dest_cloned = headers_dest.clone();

        let worker_factory = move || {
            let (headers_dest, condvar) = &*headers_dest_cloned;

            let mut encoder = encoder_factory();

            let mut headers = Vec::new();
            headers.extend_from_slice(
                encoder
                    .headers()
                    .expect("Couldn't get x264 headers")
                    .entirety(),
            );

            *headers_dest.lock() = Some(headers.into_boxed_slice());
            condvar.notify_one();

            RecordWorker {
                capturer,
                encoder,
                width,
                height,
                data_buf: data_buf,
                timebase,
                record_start_time: Instant::now(),
                buffered_frames,
            }
        };

        // the rate is infinity because it's gonna be limited by the capturer
        let thread_loop = ThreadLoop::new(worker_factory, f64::INFINITY);

        // waiting for headers from the thread with the encoder
        let (headers_lock, condvar) = &*headers_dest;
        let mut headers = headers_lock.lock();

        let headers = match headers.take() {
            Some(h) => h,
            None => {
                condvar.wait(&mut headers);
                headers.take().unwrap()
            }
        };

        Self {
            thread_loop,
            data_buf: data_buf_view,
            headers,
        }
    }

    pub fn data_buffer(&mut self) -> Result<EncodedDataGuard<'_>, RecordError> {
        // bubble up errors
        for i in self.thread_loop.work_try_iter() {
            i?;
        }

        Ok(self.data_buf.get())
    }

    pub fn headers(&self) -> &[u8] {
        &self.headers
    }

    pub fn block_until_next_flush(&self) -> Result<(), RecordError> {
        for i in self.thread_loop.work_iter() {
            match i? {
                EncodeStatus::Flushed => return Ok(()),
                _ => (),
            }
        }
        // technically unreachable unless something nasty happends
        Ok(())
    }
}

#[derive(Debug)]
pub struct CapturerSettings<F>
where
    F: FnMut() -> Display + Send + 'static,
{
    pub display_factory: F,
    pub target_rate: f64,
}

#[derive(Debug)]
pub struct BufferingSettings {
    pub buffer_capacity: usize,
    pub buffered_frames: usize,
}

pub struct EncoderSettings<F>
where
    F: FnOnce() -> Encoder + Send + 'static,
{
    pub encoder_factory: F,
    pub timebase: f64,
}

#[derive(Debug, Clone, Copy)]
pub enum EncodeStatus {
    Skipped,
    PreBuffered,
    Flushed,
}
