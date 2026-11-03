//! Bounded, cancellable background decoding. No downloaded executables or shell commands.
use anyhow::{bail, Context, Result};
use eframe::egui::{self, Color32, ColorImage, Rect, TextureHandle, TextureOptions, Vec2};
use image::{AnimationDecoder, ImageDecoder};
#[cfg(not(feature = "lite"))]
use std::process::{Command, Stdio};
use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::{BufReader, Read},
    path::{Path, PathBuf},
    process::Child,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, Sender, SyncSender},
        Arc, Mutex,
    },
    time::Duration,
};

pub fn rgba_image(image: image::DynamicImage, max_width: u32, max_height: u32) -> ColorImage {
    let image = if image.width() > max_width || image.height() > max_height {
        image.thumbnail(max_width, max_height)
    } else {
        image
    };
    let rgba = image.to_rgba8();
    ColorImage::from_rgba_unmultiplied(
        [rgba.width() as usize, rgba.height() as usize],
        rgba.as_raw(),
    )
}
pub fn decode_image(bytes: &[u8], width: u32, height: u32) -> Result<ColorImage> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    Ok(rgba_image(reader.decode()?, width, height))
}

pub fn is_video(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str(),
        "mp4" | "webm" | "mkv" | "mov" | "avi" | "m4v"
    )
}

enum MediaEvent {
    Frame(ColorImage),
    Error(String),
}
struct Worker {
    cancel: Sender<()>,
    stopped: Arc<AtomicBool>,
    animate: Arc<AtomicBool>,
    child: Arc<Mutex<Option<Child>>>,
    rx: Receiver<MediaEvent>,
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Relaxed);
        let _ = self.cancel.send(());
        if let Ok(mut slot) = self.child.lock() {
            if let Some(child) = slot.as_mut() {
                let _ = child.kill();
            }
        }
    }
}
#[derive(Default)]
pub struct Background {
    worker: Option<Worker>,
    texture: Option<TextureHandle>,
    pub status: String,
    cache: Option<std::path::PathBuf>,
    poster: Option<std::path::PathBuf>,
    poster_saved: bool,
}
impl Background {
    pub fn with_cache(root: &Path) -> Self {
        Self {
            cache: Some(root.join("media")),
            ..Default::default()
        }
    }
    pub fn load(&mut self, path: &str, animate: bool, ctx: &egui::Context) {
        self.worker = None;
        self.status.clear();
        if cfg!(feature = "lite") && is_video(Path::new(path)) {
            self.texture = None;
            self.status = "Lite поддерживает только фото и GIF, без видео/FFmpeg".into();
            return;
        }
        if path.is_empty() {
            self.texture = None;
            return;
        }
        self.poster = self.cache.as_ref().map(|root| {
            use sha2::Digest;
            root.join(format!("{:x}.png", sha2::Sha256::digest(path.as_bytes())))
        });
        self.poster_saved = false;
        if let Some(poster) = &self.poster {
            if let Ok(bytes) = std::fs::read(poster) {
                if let Ok(image) = decode_image(&bytes, 960, 540) {
                    self.texture =
                        Some(ctx.load_texture("background-poster", image, TextureOptions::LINEAR));
                    self.poster_saved = true;
                }
            }
        }
        self.status = "Загрузка фона…".into();
        let (tx, rx) = mpsc::sync_channel(2);
        let (cancel, stop) = mpsc::channel();
        let stopped = Arc::new(AtomicBool::new(false));
        let animation = Arc::new(AtomicBool::new(animate));
        let child = Arc::new(Mutex::new(None));
        self.worker = Some(Worker {
            cancel,
            stopped: stopped.clone(),
            animate: animation.clone(),
            child: child.clone(),
            rx,
        });
        let path = path.to_owned();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = if is_video(Path::new(&path)) {
                #[cfg(not(feature = "lite"))]
                {
                    video(&path, &tx, &stop, &stopped, &animation, &child, &ctx)
                }
                #[cfg(feature = "lite")]
                {
                    Err(anyhow::anyhow!("Видео отключено в Lite"))
                }
            } else if Path::new(&path)
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("gif"))
            {
                gif(&path, &tx, &stop, &stopped, &animation, &ctx)
            } else {
                still(&path, &tx, &stopped, &ctx)
            };
            if let Ok(mut slot) = child.lock() {
                if let Some(mut child) = slot.take() {
                    let _ = child.kill();
                    let _ = child.wait();
                }
            }
            if !stopped.load(Ordering::Relaxed) {
                if let Err(e) = result {
                    let _ = tx.send(MediaEvent::Error(format!("{e:#}")));
                    ctx.request_repaint();
                }
            }
        });
    }
    pub fn animate(&self, enabled: bool) {
        if let Some(worker) = &self.worker {
            worker.animate.store(enabled, Ordering::Relaxed);
        }
    }
    pub fn paint(&mut self, ui: &egui::Ui) -> bool {
        if let Some(worker) = &self.worker {
            while let Ok(event) = worker.rx.try_recv() {
                match event {
                    MediaEvent::Frame(image) => {
                        if !self.poster_saved {
                            if let Some(path) = self.poster.clone() {
                                let image = image.clone();
                                std::thread::spawn(move || {
                                    let bytes: Vec<u8> = image
                                        .pixels
                                        .iter()
                                        .flat_map(|c| c.to_srgba_unmultiplied())
                                        .collect();
                                    if let Some(image) = image::RgbaImage::from_raw(
                                        image.size[0] as u32,
                                        image.size[1] as u32,
                                        bytes,
                                    ) {
                                        let image = image::DynamicImage::ImageRgba8(image)
                                            .thumbnail(960, 540);
                                        let mut encoded = std::io::Cursor::new(Vec::new());
                                        if image
                                            .write_to(&mut encoded, image::ImageFormat::Png)
                                            .is_ok()
                                        {
                                            let _ = crate::downloads::atomic_write(
                                                &path,
                                                &encoded.into_inner(),
                                            );
                                        }
                                    }
                                });
                            }
                            self.poster_saved = true;
                        }
                        if let Some(texture) = &mut self.texture {
                            texture.set(image, TextureOptions::LINEAR);
                        } else {
                            self.texture = Some(ui.ctx().load_texture(
                                "custom-background",
                                image,
                                TextureOptions::LINEAR,
                            ));
                        }
                        self.status.clear();
                    }
                    MediaEvent::Error(error) => self.status = error,
                }
            }
        }
        if let Some(texture) = &self.texture {
            let rect = ui.max_rect();
            let uv = cover_uv(texture.size_vec2(), rect.size());
            ui.painter().image(texture.id(), rect, uv, Color32::WHITE);
            true
        } else {
            false
        }
    }
}

/// Copy selected wallpaper once into writable launcher data; never move/delete the original.
pub fn import_background(source: &Path, root: &Path) -> Result<String> {
    anyhow::ensure!(
        !cfg!(feature = "lite") || !is_video(source),
        "Lite поддерживает только фото и GIF"
    );
    use sha2::Digest;
    let metadata = source.metadata().context("Файл фона не найден")?;
    if !metadata.is_file() || metadata.len() > 4 * 1024 * 1024 * 1024 {
        bail!("Фон должен быть файлом не больше 4 ГиБ");
    }
    let extension = source
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !matches!(
        extension.as_str(),
        "png"
            | "jpg"
            | "jpeg"
            | "webp"
            | "bmp"
            | "gif"
            | "mp4"
            | "webm"
            | "mkv"
            | "mov"
            | "avi"
            | "m4v"
    ) {
        bail!("Неподдерживаемый формат фона");
    }
    let id = format!(
        "{}:{}:{:?}",
        source.display(),
        metadata.len(),
        metadata.modified()?
    );
    let dir = root.join("media");
    std::fs::create_dir_all(&dir)?;
    let target = dir.join(format!(
        "wallpaper-{:x}.{extension}",
        sha2::Sha256::digest(id.as_bytes())
    ));
    if !target.is_file() {
        let mut input = File::open(source)?;
        let mut file = tempfile::NamedTempFile::new_in(&dir)?;
        std::io::copy(&mut input, &mut file)?;
        file.as_file().sync_all()?;
        file.persist(&target)?;
    }
    Ok(target.to_string_lossy().into_owned())
}

pub fn cover_uv(image: Vec2, screen: Vec2) -> Rect {
    let image_ratio = image.x / image.y.max(1.);
    let screen_ratio = screen.x / screen.y.max(1.);
    let (x, y) = if image_ratio > screen_ratio {
        (screen_ratio / image_ratio, 1.)
    } else {
        (1., image_ratio / screen_ratio)
    };
    Rect::from_center_size(egui::pos2(0.5, 0.5), Vec2::new(x, y))
}
fn send_frame(tx: &SyncSender<MediaEvent>, image: ColorImage, ctx: &egui::Context) {
    let _ = tx.try_send(MediaEvent::Frame(image));
    ctx.request_repaint();
}
fn still(
    path: &str,
    tx: &SyncSender<MediaEvent>,
    stopped: &AtomicBool,
    ctx: &egui::Context,
) -> Result<()> {
    let file = File::open(path).context("Не удалось открыть фон")?;
    if file.metadata()?.len() > 64 * 1024 * 1024 {
        bail!("Картинка превышает 64 МиБ");
    }
    let mut bytes = vec![];
    file.take(64 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
    let image = if cfg!(feature = "lite") {
        decode_image(&bytes, 1280, 720)?
    } else {
        decode_image(&bytes, 1920, 1080)?
    };
    if !stopped.load(Ordering::Relaxed) {
        send_frame(tx, image, ctx);
    }
    Ok(())
}
fn wait(
    stop: &Receiver<()>,
    stopped: &AtomicBool,
    animation: &AtomicBool,
    delay: Duration,
) -> bool {
    if stopped.load(Ordering::Relaxed)
        || !matches!(
            stop.recv_timeout(delay),
            Err(mpsc::RecvTimeoutError::Timeout)
        )
    {
        return false;
    }
    while !animation.load(Ordering::Relaxed) {
        if stopped.load(Ordering::Relaxed)
            || !matches!(
                stop.recv_timeout(Duration::from_millis(100)),
                Err(mpsc::RecvTimeoutError::Timeout)
            )
        {
            return false;
        }
    }
    true
}
fn gif(
    path: &str,
    tx: &SyncSender<MediaEvent>,
    stop: &Receiver<()>,
    stopped: &AtomicBool,
    animation: &AtomicBool,
    ctx: &egui::Context,
) -> Result<()> {
    loop {
        let file = File::open(path)?;
        if file.metadata()?.len() > 64 * 1024 * 1024 {
            bail!("GIF превышает 64 МиБ");
        }
        let mut decoder = image::codecs::gif::GifDecoder::new(BufReader::new(file))?;
        let (w, h) = decoder.dimensions();
        if w > 2048 || h > 2048 {
            bail!("GIF должен быть не больше 2048×2048");
        }
        let mut limits = image::Limits::default();
        limits.max_alloc = Some(64 * 1024 * 1024);
        decoder.set_limits(limits)?;
        let mut count = 0;
        for frame in decoder.into_frames() {
            if stopped.load(Ordering::Relaxed) {
                return Ok(());
            }
            let frame = frame?;
            let (n, d) = frame.delay().numer_denom_ms();
            let delay = Duration::from_millis(
                (n as u64 / d.max(1) as u64)
                    .clamp(if cfg!(feature = "lite") { 67 } else { 20 }, 60_000),
            );
            send_frame(
                tx,
                rgba_image(
                    image::DynamicImage::ImageRgba8(frame.into_buffer()),
                    if cfg!(feature = "lite") { 960 } else { 1280 },
                    if cfg!(feature = "lite") { 540 } else { 720 },
                ),
                ctx,
            );
            count += 1;
            if !wait(stop, stopped, animation, delay) {
                return Ok(());
            }
        }
        if count == 0 {
            bail!("GIF не содержит кадров");
        }
    }
}
#[cfg(not(feature = "lite"))]
fn video(
    path: &str,
    tx: &SyncSender<MediaEvent>,
    stop: &Receiver<()>,
    stopped: &AtomicBool,
    animation: &AtomicBool,
    slot: &Mutex<Option<Child>>,
    ctx: &egui::Context,
) -> Result<()> {
    if !Path::new(path).is_file() {
        bail!("Видеофайл не найден");
    }
    let mut command = Command::new("ffmpeg");
    command
        .args([
            "-nostdin",
            "-loglevel",
            "error",
            "-threads",
            "2",
            "-filter_threads",
            "1",
            "-protocol_whitelist",
            "file,pipe",
            "-stream_loop",
            "-1",
            "-i",
        ])
        .arg(path)
        .args([
            "-an",
            "-sn",
            "-vf",
            "fps=24,scale=1280:720:force_original_aspect_ratio=increase,crop=1280:720",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            "pipe:1",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child=command.spawn().context("Для видео установите FFmpeg и добавьте ffmpeg.exe в PATH. Картинки и GIF работают без FFmpeg")?;
    let mut stdout = child.stdout.take().context("Не удалось прочитать видео")?;
    {
        let mut guard = slot
            .lock()
            .map_err(|_| anyhow::anyhow!("Ошибка процесса FFmpeg"))?;
        if stopped.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(());
        }
        *guard = Some(child);
    }
    let mut frame = vec![0u8; 1280 * 720 * 4];
    while !stopped.load(Ordering::Relaxed) {
        if stdout.read_exact(&mut frame).is_err() {
            if stopped.load(Ordering::Relaxed) {
                return Ok(());
            }
            bail!("FFmpeg не смог воспроизвести видео. Проверьте файл и кодек.");
        }
        send_frame(
            tx,
            ColorImage::from_rgba_unmultiplied([1280, 720], &frame),
            ctx,
        );
        if !wait(stop, stopped, animation, Duration::from_millis(42)) {
            return Ok(());
        }
    }
    Ok(())
}

#[derive(Default)]
pub struct Thumbnails {
    cache: Option<PathBuf>,
    textures: HashMap<String, TextureHandle>,
    epoch: Arc<AtomicU64>,
    rx: Option<Receiver<(String, ColorImage)>>,
}
impl Thumbnails {
    pub fn set_cache(&mut self, path: PathBuf) {
        self.cache = Some(path);
    }
    pub fn request(&mut self, urls: Vec<String>, ctx: &egui::Context) {
        if cfg!(feature = "lite") {
            return;
        }
        let epoch = self.epoch.fetch_add(1, Ordering::Relaxed) + 1;
        let generation = self.epoch.clone();
        if self.textures.len() > 100 {
            self.textures.clear();
        }
        let urls: HashSet<_> = urls
            .into_iter()
            .filter(|url| !url.is_empty() && !self.textures.contains_key(url))
            .collect();
        let (tx, rx) = mpsc::sync_channel(2);
        self.rx = Some(rx);
        let ctx = ctx.clone();
        let cache = self.cache.clone();
        std::thread::spawn(move || {
            let Ok(client) = reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(12))
                .redirect(reqwest::redirect::Policy::none())
                .user_agent("MLauncher/0.1.0")
                .build()
            else {
                return;
            };
            for url in urls {
                if generation.load(Ordering::Relaxed) != epoch {
                    return;
                }
                let cached = cache.as_ref().map(|root| {
                    use sha2::Digest;
                    root.join(format!("{:x}.png", sha2::Sha256::digest(url.as_bytes())))
                });
                let image = cached
                    .as_ref()
                    .and_then(|path| std::fs::read(path).ok())
                    .filter(|b| b.len() < 1024 * 1024)
                    .and_then(|bytes| decode_image(&bytes, 96, 96).ok());
                if let Ok(image) = image
                    .map(Ok)
                    .unwrap_or_else(|| download_icon(&client, &url))
                {
                    if let Some(path) = cached {
                        if !path.exists() {
                            if let Some(parent) = path.parent() {
                                let _ = std::fs::create_dir_all(parent);
                            }
                            let bytes: Vec<u8> = image
                                .pixels
                                .iter()
                                .flat_map(|c| c.to_srgba_unmultiplied())
                                .collect();
                            let _ = image::save_buffer(
                                &path,
                                &bytes,
                                image.size[0] as u32,
                                image.size[1] as u32,
                                image::ColorType::Rgba8,
                            );
                        }
                    }
                    if tx.send((url, image)).is_err() {
                        return;
                    }
                    ctx.request_repaint();
                }
            }
        });
    }
    pub fn drain(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.rx {
            while let Ok((url, image)) = rx.try_recv() {
                self.textures.insert(
                    url.clone(),
                    ctx.load_texture(url, image, TextureOptions::LINEAR),
                );
            }
        }
    }
    pub fn get(&self, url: &str) -> Option<&TextureHandle> {
        self.textures.get(url)
    }
}
impl Drop for Thumbnails {
    fn drop(&mut self) {
        self.epoch.fetch_add(1, Ordering::Relaxed);
    }
}
pub(crate) fn download_icon(client: &reqwest::blocking::Client, url: &str) -> Result<ColorImage> {
    let parsed = reqwest::Url::parse(url)?;
    if parsed.scheme() != "https" || !parsed.host_str().is_some_and(icon_host_allowed) {
        bail!("Недопустимый CDN иконки");
    }
    let response = client.get(parsed).send()?.error_for_status()?;
    let mut bytes = vec![];
    response.take(4 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > 4 * 1024 * 1024 {
        bail!("Иконка слишком большая");
    }
    decode_image(&bytes, 96, 96)
}

fn icon_host_allowed(host: &str) -> bool {
    matches!(
        host,
        "cdn.modrinth.com" | "cdn.atlcdn.net" | "cdn.feed-the-beast.com" | "cdn.technicpack.net"
    ) || host.ends_with(".forgecdn.net")
}

pub fn check_catalog_icon(url: &str) -> Result<[usize; 2]> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    Ok(download_icon(&client, url)?.size)
}

/// Local end-to-end decoder check. Generates media only inside an automatically removed tempdir.
pub fn smoke_test(icon_url: &str) -> Result<()> {
    let icon = download_icon(
        &reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()?,
        icon_url,
    )?;
    println!(
        "Mod icon downloaded and decoded: {}x{}",
        icon.size[0], icon.size[1]
    );
    let root = tempfile::tempdir()?;
    let png = root.path().join("background.png");
    image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
        64,
        32,
        image::Rgba([10, 150, 100, 255]),
    ))
    .save(&png)?;
    let gif_path = root.path().join("background.gif");
    let mut encoder = image::codecs::gif::GifEncoder::new(File::create(&gif_path)?);
    for color in [[255, 0, 0, 255], [0, 255, 0, 255]] {
        encoder.encode_frame(image::Frame::new(image::RgbaImage::from_pixel(
            32,
            32,
            image::Rgba(color),
        )))?;
    }
    drop(encoder);
    let (tx, rx) = mpsc::sync_channel(2);
    let (cancel, stop) = mpsc::channel();
    cancel.send(())?;
    still(
        png.to_str().context("Path")?,
        &tx,
        &AtomicBool::new(false),
        &egui::Context::default(),
    )?;
    assert!(matches!(
        rx.recv_timeout(Duration::from_secs(3))?,
        MediaEvent::Frame(_)
    ));
    gif(
        gif_path.to_str().context("Path")?,
        &tx,
        &stop,
        &AtomicBool::new(false),
        &AtomicBool::new(true),
        &egui::Context::default(),
    )?;
    assert!(matches!(
        rx.recv_timeout(Duration::from_secs(3))?,
        MediaEvent::Frame(_)
    ));
    println!("Native PNG and GIF decoding OK");
    #[cfg(not(feature = "lite"))]
    {
        if Command::new("ffmpeg")
            .arg("-version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_err()
        {
            println!("FFmpeg not installed: video smoke test skipped");
            return Ok(());
        }
        let mp4 = root.path().join("background.mp4");
        let output = Command::new("ffmpeg")
            .args([
                "-nostdin",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=c=blue:s=64x64:r=24",
                "-t",
                "1",
                "-pix_fmt",
                "yuv420p",
            ])
            .arg(&mp4)
            .output()?;
        if !output.status.success() {
            bail!("Could not generate video fixture");
        }
        let mut background = Background::default();
        background.load(
            mp4.to_str().context("Path")?,
            true,
            &egui::Context::default(),
        );
        let worker = background.worker.as_ref().context("Video worker missing")?;
        match worker.rx.recv_timeout(Duration::from_secs(20))? {
            MediaEvent::Frame(frame) => assert_eq!(frame.size, [1280, 720]),
            MediaEvent::Error(e) => bail!("{e}"),
        }
        background.animate(false);
        // Cancellation kills the child and lets its decoding thread reap it.
        if let Some(worker) = background.worker.as_ref() {
            worker.stopped.store(true, Ordering::Relaxed);
            if let Ok(mut slot) = worker.child.lock() {
                if let Some(mut process) = slot.take() {
                    let _ = process.kill();
                    let _ = process.wait();
                }
            }
        }
        drop(background);
        println!("FFmpeg MP4 decoding, pause and cancellation OK");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[cfg(feature = "lite")]
    fn lite_rejects_video_without_starting_a_decoder() {
        let ctx = egui::Context::default();
        let mut background = Background::default();
        background.load("missing.mp4", true, &ctx);
        assert!(background.worker.is_none());
        assert!(!background.status.is_empty());
        let root = tempfile::tempdir().unwrap();
        assert!(import_background(Path::new("missing.mp4"), root.path()).is_err());
    }
    #[test]
    #[cfg(not(feature = "lite"))]
    fn cached_poster_exists_before_decoder_returns() {
        use sha2::Digest;
        let root = tempfile::tempdir().unwrap();
        let source = root
            .path()
            .join("not-yet-decoded.mp4")
            .to_string_lossy()
            .into_owned();
        let poster = root
            .path()
            .join("media")
            .join(format!("{:x}.png", sha2::Sha256::digest(source.as_bytes())));
        std::fs::create_dir_all(poster.parent().unwrap()).unwrap();
        image::RgbaImage::from_pixel(32, 16, image::Rgba([10, 20, 30, 255]))
            .save(&poster)
            .unwrap();
        let mut background = Background::with_cache(root.path());
        background.load(&source, true, &egui::Context::default());
        assert!(background.texture.is_some());
        assert!(background.poster_saved);
    }
    #[test]
    fn imports_wallpaper_without_removing_original() {
        let root = tempfile::tempdir().unwrap();
        let original = root.path().join("chosen.png");
        image::RgbaImage::new(16, 16).save(&original).unwrap();
        let managed = import_background(&original, root.path()).unwrap();
        assert!(original.is_file());
        assert!(Path::new(&managed).starts_with(root.path().join("media")));
        assert_eq!(
            std::fs::read(original).unwrap(),
            std::fs::read(managed).unwrap()
        );
    }
    #[test]
    fn crop_does_not_stretch() {
        let uv = cover_uv(Vec2::new(1920., 1080.), Vec2::new(1000., 1000.));
        assert!(uv.width() < 1.);
        assert_eq!(uv.height(), 1.);
    }
    #[test]
    fn video_extensions_and_image_limits() {
        assert!(is_video(Path::new("wall.MP4")));
        assert!(!is_video(Path::new("wall.gif")));
        assert!(decode_image(b"not an image", 96, 96).is_err());
    }
    #[test]
    fn native_gif_stream_and_still_image() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.gif");
        let mut encoder = image::codecs::gif::GifEncoder::new(File::create(&path).unwrap());
        encoder
            .encode_frame(image::Frame::new(image::RgbaImage::from_pixel(
                2,
                2,
                image::Rgba([255, 0, 0, 255]),
            )))
            .unwrap();
        drop(encoder);
        let (tx, rx) = mpsc::sync_channel(2);
        let (cancel, stop) = mpsc::channel();
        cancel.send(()).unwrap();
        gif(
            path.to_str().unwrap(),
            &tx,
            &stop,
            &AtomicBool::new(false),
            &AtomicBool::new(true),
            &egui::Context::default(),
        )
        .unwrap();
        assert!(matches!(rx.try_recv(), Ok(MediaEvent::Frame(_))));
        let image = image::DynamicImage::new_rgba8(4, 4);
        let png = dir.path().join("image.png");
        image.save(&png).unwrap();
        still(
            png.to_str().unwrap(),
            &tx,
            &AtomicBool::new(false),
            &egui::Context::default(),
        )
        .unwrap();
        assert!(matches!(rx.try_recv(), Ok(MediaEvent::Frame(_))));
    }
    #[test]
    fn gif_can_pause_resume_and_cancel() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("animation.gif");
        let mut encoder = image::codecs::gif::GifEncoder::new(File::create(&path).unwrap());
        for color in [[255, 0, 0, 255], [0, 255, 0, 255]] {
            encoder
                .encode_frame(image::Frame::new(image::RgbaImage::from_pixel(
                    2,
                    2,
                    image::Rgba(color),
                )))
                .unwrap();
        }
        drop(encoder);
        let mut background = Background::default();
        background.load(path.to_str().unwrap(), false, &egui::Context::default());
        let rx = &background.worker.as_ref().unwrap().rx;
        assert!(matches!(
            rx.recv_timeout(Duration::from_secs(3)),
            Ok(MediaEvent::Frame(_))
        ));
        assert!(matches!(
            rx.recv_timeout(Duration::from_millis(150)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        background.animate(true);
        assert!(matches!(
            rx.recv_timeout(Duration::from_secs(3)),
            Ok(MediaEvent::Frame(_))
        ));
        drop(background);
    }
}
