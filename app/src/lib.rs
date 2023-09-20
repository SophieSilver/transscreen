use std::{time::{Instant, Duration}, fs::File, io::{BufWriter, Write}};

use scrap::Display;
use screen_cap::record::{CapturerSettings, Recorder, BufferingSettings, EncoderSettings};
use x264::{Setup, Colorspace, Preset, Tune};

const TARGET_RATE: f64 = 60.0;
// 50 MiB
const BUFFER_CAPACITY: usize = 50 * 8 * 1024 * 1024;
const BUFFERED_FRAMES: usize = 29;
// 4 Mbits/s
const BITRATE: i32 = 4000;
const TIMEBASE: f64 = 1000.0;

const PRESET: Preset = Preset::Ultrafast;
const TUNE: Tune = Tune::StillImage;
const FAST_DECODE: bool = true;
const ZERO_LATENCY: bool = true;

pub fn run() {
    record_to_file();
}

fn record_to_file() {
    let display = Display::primary().unwrap();
    let width = display.width();
    let height = display.height();
    
    let capturer_settings = CapturerSettings {
        display_factory: || Display::primary().unwrap(),
        target_rate: TARGET_RATE,
    };
    
    let buffering_settings = BufferingSettings {
        buffer_capacity: BUFFER_CAPACITY,
        buffered_frames: BUFFERED_FRAMES,
    };
    
    let encoder_settings = EncoderSettings {
        encoder_factory: move || {
            Setup::preset(PRESET, TUNE, FAST_DECODE, ZERO_LATENCY)
                .bitrate(BITRATE)
                .timebase(TIMEBASE as u32, 1)
                .build(Colorspace::BGRA, width as _, height as _)
                .unwrap()
        },
        timebase: TIMEBASE,
    };
    
    let file = File::create("thing.h264").unwrap();
    let mut file_buf = BufWriter::with_capacity(8 * 1024 * 1024, file);

    let mut recorder = Recorder::new(capturer_settings, buffering_settings, encoder_settings);
    
    let mut last_chunk_id = 0;
    
    let start_time = Instant::now();
    
    file_buf.write_all(recorder.headers()).unwrap();
    
    while start_time.elapsed() < Duration::from_secs(60) {
        recorder.block_until_next_flush().unwrap();
        
        let data_buf = recorder.data_buffer().unwrap();
        let (id_min, id_max) = data_buf.id_bounds();
        
        let start_id = id_min.max(last_chunk_id);
        
        for i in start_id..id_max {
            let frame = data_buf.get(i).unwrap();
            file_buf.write_all(frame.data()).unwrap();
        }
        
        last_chunk_id = id_max;
    }
    file_buf.flush().unwrap();
}
