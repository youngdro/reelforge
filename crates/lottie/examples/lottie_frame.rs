//! Draws frames of a Lottie file as raw RGBA: `lottie_frame FILE WIDTH HEIGHT OUT FRAME…`
//! writes `OUT.<frame>.rgba` for each frame and says how long each took.

use reelforge_lottie::{Animation, Bitmap, Files};

struct NoFiles;

impl Files for NoFiles {
    fn decode(&self, _: &[u8]) -> Option<Bitmap> {
        None
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 5 {
        eprintln!("usage: lottie_frame FILE WIDTH HEIGHT OUT FRAME…");
        std::process::exit(2);
    }
    let bytes = std::fs::read(&args[1]).expect("read the file");
    let anim = match Animation::open(&bytes, &NoFiles) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    let (w, h): (u32, u32) = (args[2].parse().expect("width"), args[3].parse().expect("height"));
    println!(
        "{} x {}, {} fps, frames {} to {}; not drawn: {:?}",
        anim.width,
        anim.height,
        anim.fps,
        anim.first,
        anim.end,
        anim.unsupported()
    );
    for frame in &args[5..] {
        let f: f32 = frame.parse().expect("frame");
        let started = std::time::Instant::now();
        let rgba = anim.render(f, w, h);
        println!("frame {f}: {:.1} ms", started.elapsed().as_secs_f64() * 1000.0);
        std::fs::write(format!("{}.{frame}.rgba", args[4]), rgba).expect("write the picture");
    }
}
