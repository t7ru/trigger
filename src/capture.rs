pub fn screenshot_png() -> Result<Vec<u8>, String> {
    let monitors = xcap::Monitor::all().map_err(|e| e.to_string())?;
    if monitors.is_empty() {
        return Err("no monitor".into());
    }
    let primary = monitors
        .iter()
        .position(|m| m.is_primary().unwrap_or(false))
        .unwrap_or(0);
    let shot = monitors[primary]
        .capture_image()
        .map_err(|e| e.to_string())?;
    let (w, h) = (shot.width(), shot.height());
    let img = image::RgbaImage::from_raw(w, h, shot.into_raw()).ok_or("bad screenshot")?;
    let small = image::DynamicImage::ImageRgba8(img)
        .thumbnail(1600, 1600)
        .to_rgb8();
    let mut png = Vec::new();
    small
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    Ok(png)
}
