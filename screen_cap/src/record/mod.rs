pub mod encoded_buffer;

use std::time::Instant;

use scrap::Display;
use thiserror::Error;
use utils::multibuffer::MultiBuffer;
use x264::{Data, Encoder, Picture, Image};

use crate::{capture::ThreadedCapturer, frame::FrameError};

struct RecordWorker {
    capturer: ThreadedCapturer,
    encoder: Encoder,
    width: i32,
    height: i32,
    // TODO: change the data structure
    data_buf: (),
    timebase: f64,
    record_start_time: Instant,
}

impl RecordWorker {
    fn update(&mut self) -> Result<Picture, RecordError> {
        // get the frame
        let frame = self.capturer.frame()?;

        let frame_data = if cfg!(target_os = "macos") {
            // stride is different on macos
            // https://github.com/quadrupleslap/scrap/issues/44#issuecomment-1486345836
            let w = self.width as usize;
            let h = self.height as usize;
            
            &frame[..w * h * 4]
        } else {
            &frame
        };
        
        let image = Image::bgra(self.width, self.width, frame_data);
        
        // actually encoding
        let elapsed = self.record_start_time.elapsed().as_secs_f64();
        let timestamp = (elapsed * self.timebase) as i64; 
        let (data, picture) = self.encoder.encode(timestamp, image)?;
        
        // update the buffer
        // let back_buf = self.data_buf.back_mut();
        // back_buf.clear();
        // back_buf.extend_from_slice(data.entirety());
        // self.data_buf.swap();
        
        // Ok(picture)
        todo!()
    }
}

#[derive(Debug, Error)]
pub enum RecordError {
    #[error(transparent)]
    FrameError(#[from] FrameError),
    // x264::Error is zero sized and doesn't even implement the Error trait
    #[error("there has been an error while encoding a frame")]
    EncodeError
}

// can't do this with a macro because x264::Error doesn't implement the Error trait
impl From<x264::Error> for RecordError {
    fn from(_: x264::Error) -> Self {
        Self::EncodeError
    }
}
