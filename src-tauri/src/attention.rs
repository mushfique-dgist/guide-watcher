//! Tells the user when Guide Watcher wants them.
//!
//! Two signals, because a guide takes hours and nobody watches the window that long:
//! a desktop notification at the moment something happens, and a count drawn on the taskbar
//! icon that stays until the user looks at the window. The count is what messaging apps put
//! there; on Windows it is an overlay icon, which is why the badge is rendered here.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

use fontdue::{Font, FontSettings};
use tauri::image::Image;
use tauri::{AppHandle, Manager};
use tauri_plugin_notification::NotificationExt;

const APP_NAME: &str = "Guide Watcher";
const MAIN_WINDOW: &str = "main";
/// Matches the id the tray is built with in `lib.rs`.
const TRAY_ID: &str = "main-tray";
/// Side of the square badge. Windows asks for a 16px overlay and scales it, so draw it larger.
/// Only Windows has a taskbar overlay to put it on; elsewhere the count lives on the tray icon.
#[cfg_attr(not(windows), allow(dead_code))]
const BADGE_PX: u32 = 32;
/// Opaque red disc behind the number.
const BADGE_FILL: [u8; 3] = [211, 47, 47];
const BADGE_INK: [u8; 3] = [255, 255, 255];
/// Above this the badge reads "9+" rather than a number nobody counts.
const BADGE_MAX: usize = 9;

/// Endings the user has not acknowledged yet. Cleared when the window takes focus.
static UNSEEN: AtomicUsize = AtomicUsize::new(0);
/// Set while the user's own stop is working through the running jobs, so that the endings it
/// causes raise nothing. Cleared when a job starts again.
static USER_STOPPED: AtomicBool = AtomicBool::new(false);

/// What each running job is writing, so its ending can name the guide. `done` carries only a
/// job id, and the name is worth more than a status line.
fn guide_names() -> &'static Mutex<HashMap<String, String>> {
    static NAMES: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    NAMES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Record the guide a job is writing. Safe to call more than once for the same job.
pub(crate) fn remember_guide(job_id: &str, guide: &str) {
    if let Ok(mut names) = guide_names().lock() {
        names.insert(job_id.to_string(), guide_display_name(guide));
    }
}

/// A job is starting, so any earlier stop is over.
pub(crate) fn job_started(job_id: &str) {
    let _ = job_id;
    USER_STOPPED.store(false, Ordering::SeqCst);
}

/// The user asked to stop. Endings that follow are expected and raise nothing.
pub(crate) fn user_stopped_jobs() {
    USER_STOPPED.store(true, Ordering::SeqCst);
}

/// A job ended: notify, and count it unless the user is already looking at the window.
pub(crate) fn job_finished(app: &AppHandle, job_id: &str, exit_code: i32, wrote_output: bool) {
    let guide = guide_names()
        .lock()
        .ok()
        .and_then(|mut names| names.remove(job_id));
    if USER_STOPPED.load(Ordering::SeqCst) {
        return;
    }
    let (title, body) = ending_message(guide.as_deref(), exit_code, wrote_output);
    notify(app, &title, &body);
    if !user_is_watching(app) {
        bump(app, 1);
    }
}

/// Lecture files are waiting to be reviewed.
pub(crate) fn sources_waiting(app: &AppHandle, count: usize) {
    if count == 0 {
        return;
    }
    let (title, body) = sources_message(count);
    notify(app, &title, &body);
    if !user_is_watching(app) {
        bump(app, 1);
    }
}

/// The user is looking at the window, so nothing is unseen any more.
pub(crate) fn clear(app: &AppHandle) {
    if UNSEEN.swap(0, Ordering::SeqCst) != 0 {
        paint_badge(app, 0);
    }
}

fn bump(app: &AppHandle, by: usize) {
    let total = UNSEEN.fetch_add(by, Ordering::SeqCst) + by;
    paint_badge(app, total);
}

/// Whether the window is in front of the user right now.
fn user_is_watching(app: &AppHandle) -> bool {
    app.get_webview_window(MAIN_WINDOW).is_some_and(|window| {
        window.is_focused().unwrap_or(false)
            && window.is_visible().unwrap_or(false)
            && !window.is_minimized().unwrap_or(false)
    })
}

fn notify(app: &AppHandle, title: &str, body: &str) {
    if let Err(error) = app.notification().builder().title(title).body(body).show() {
        // A refused notification must never take a finished guide down with it.
        eprintln!("Guide Watcher could not show a desktop notification: {error}");
    }
}

fn paint_badge(app: &AppHandle, count: usize) {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
        #[cfg(target_os = "windows")]
        {
            // Windows has no numeric badge: the taskbar shows a small overlay icon instead.
            let _ = window.set_overlay_icon(badge_icon(count));
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = window.set_badge_count((count > 0).then_some(count as i64));
        }
    }
    // Closing the window hides it in the tray, and a hidden window has no taskbar button to
    // badge - which is exactly when a long run tends to finish.
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = tray.set_tooltip(Some(tray_tooltip(count)));
        if let Some(icon) = tray_icon(app, count) {
            let _ = tray.set_icon(Some(icon));
        }
    }
}

fn tray_tooltip(count: usize) -> String {
    match count {
        0 => APP_NAME.to_string(),
        1 => format!("{APP_NAME} - 1 update"),
        many => format!("{APP_NAME} - {many} updates"),
    }
}

/// The app icon, with the count on its lower-right corner when there is one.
fn tray_icon(app: &AppHandle, count: usize) -> Option<Image<'static>> {
    let base = app.default_window_icon()?;
    let (width, height) = (base.width(), base.height());
    let mut pixels = base.rgba().to_vec();
    if pixels.len() != (width as usize) * (height as usize) * 4 {
        return None;
    }
    let Some(badge) = badge_image(count, (width.min(height) * 6 / 10).max(8)) else {
        // Count zero: the plain app icon, restored.
        return Some(Image::new_owned(pixels, width, height));
    };
    let badge_side = badge.width();
    let origin_x = width.saturating_sub(badge_side);
    let origin_y = height.saturating_sub(badge_side);
    let badge_rgba = badge.rgba();
    for y in 0..badge_side {
        for x in 0..badge_side {
            let source = ((y * badge_side + x) * 4) as usize;
            let alpha = f32::from(badge_rgba[source + 3]) / 255.0;
            if alpha <= 0.0 {
                continue;
            }
            let target = (((origin_y + y) * width + origin_x + x) * 4) as usize;
            for channel in 0..3 {
                let under = f32::from(pixels[target + channel]);
                let over = f32::from(badge_rgba[source + channel]);
                pixels[target + channel] = (under + (over - under) * alpha).round() as u8;
            }
            pixels[target + 3] = pixels[target + 3].max(badge_rgba[source + 3]);
        }
    }
    Some(Image::new_owned(pixels, width, height))
}

/// The title and body for a job that ended.
fn ending_message(guide: Option<&str>, exit_code: i32, wrote_output: bool) -> (String, String) {
    let succeeded = exit_code == 0 && wrote_output;
    match (succeeded, guide) {
        (true, Some(name)) => (
            "Guide ready".to_string(),
            format!("{name} is finished and saved."),
        ),
        (true, None) => (
            "Guide ready".to_string(),
            "A guide is finished and saved.".to_string(),
        ),
        (false, Some(name)) => (
            "Guide Watcher needs you".to_string(),
            format!("{name} stopped before it was finished. Open Guide Watcher to see why."),
        ),
        (false, None) => (
            "Guide Watcher needs you".to_string(),
            "A guide stopped before it was finished. Open Guide Watcher to see why.".to_string(),
        ),
    }
}

fn sources_message(count: usize) -> (String, String) {
    let subject = if count == 1 {
        "1 new lecture file is".to_string()
    } else {
        format!("{count} new lecture files are")
    };
    (
        "Sources ready to review".to_string(),
        format!("{subject} waiting in Workspace."),
    )
}

/// The guide's file name, without the directory, for a one-line notification.
fn guide_display_name(guide: &str) -> String {
    guide
        .rsplit(['\\', '/'])
        .find(|part| !part.is_empty())
        .unwrap_or(guide)
        .to_string()
}

fn badge_label(count: usize) -> String {
    if count > BADGE_MAX {
        format!("{BADGE_MAX}+")
    } else {
        count.to_string()
    }
}

fn badge_font() -> &'static Font {
    static FONT: OnceLock<Font> = OnceLock::new();
    FONT.get_or_init(|| {
        Font::from_bytes(notosans::BOLD_TTF, FontSettings::default())
            .expect("the bundled Noto Sans Bold font must parse")
    })
}

/// A red disc carrying the count at the taskbar's size, or `None` when there is nothing to show.
#[cfg_attr(not(windows), allow(dead_code))]
fn badge_icon(count: usize) -> Option<Image<'static>> {
    badge_image(count, BADGE_PX)
}

/// A red disc carrying the count, drawn at `side` pixels so every size stays sharp.
fn badge_image(count: usize, side_px: u32) -> Option<Image<'static>> {
    if count == 0 || side_px == 0 {
        return None;
    }
    let side = side_px as usize;
    let mut pixels = vec![0u8; side * side * 4];
    let center = side_px as f32 / 2.0;
    let radius = center - 0.5;
    // 3x3 supersampling keeps the disc's edge smooth at taskbar size.
    for y in 0..side {
        for x in 0..side {
            let mut covered = 0u32;
            for sub_y in 0..3 {
                for sub_x in 0..3 {
                    let sample_x = x as f32 + (sub_x as f32 + 0.5) / 3.0;
                    let sample_y = y as f32 + (sub_y as f32 + 0.5) / 3.0;
                    let dx = sample_x - center;
                    let dy = sample_y - center;
                    if dx * dx + dy * dy <= radius * radius {
                        covered += 1;
                    }
                }
            }
            if covered > 0 {
                let offset = (y * side + x) * 4;
                pixels[offset] = BADGE_FILL[0];
                pixels[offset + 1] = BADGE_FILL[1];
                pixels[offset + 2] = BADGE_FILL[2];
                pixels[offset + 3] = (covered * 255 / 9) as u8;
            }
        }
    }
    draw_badge_label(&mut pixels, side_px, &badge_label(count));
    Some(Image::new_owned(pixels, side_px, side_px))
}

/// Centre the label on the disc in white.
fn draw_badge_label(pixels: &mut [u8], side_px: u32, label: &str) {
    let font = badge_font();
    // One digit can be large; "9+" has to fit two glyphs across the same disc.
    let fraction = if label.chars().count() > 1 { 0.53 } else { 0.72 };
    let size = side_px as f32 * fraction;
    let glyphs: Vec<_> = label
        .chars()
        .map(|character| font.rasterize(character, size))
        .collect();
    let total_advance: f32 = glyphs
        .iter()
        .map(|(metrics, _)| metrics.advance_width)
        .sum();
    let cap_height = glyphs
        .iter()
        .map(|(metrics, _)| metrics.height as i32 + metrics.ymin)
        .max()
        .unwrap_or(0);
    let side = side_px as i32;
    let mut pen_x = (side_px as f32 - total_advance) / 2.0;
    let baseline = (side + cap_height) / 2;
    for (metrics, bitmap) in &glyphs {
        let left = pen_x.round() as i32 + metrics.xmin;
        let top = baseline - metrics.height as i32 - metrics.ymin;
        for row in 0..metrics.height {
            for column in 0..metrics.width {
                let coverage = bitmap[row * metrics.width + column];
                if coverage == 0 {
                    continue;
                }
                let x = left + column as i32;
                let y = top + row as i32;
                if x < 0 || y < 0 || x >= side || y >= side {
                    continue;
                }
                let offset = ((y * side + x) * 4) as usize;
                let alpha = f32::from(coverage) / 255.0;
                for channel in 0..3 {
                    let under = f32::from(pixels[offset + channel]);
                    let over = f32::from(BADGE_INK[channel]);
                    pixels[offset + channel] = (under + (over - under) * alpha).round() as u8;
                }
                pixels[offset + 3] = pixels[offset + 3].max(coverage);
            }
        }
        pen_x += metrics.advance_width;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_finished_guide_and_a_failure_read_differently_and_name_the_guide() {
        let (title, body) = ending_message(Some("L4 Guide.md"), 0, true);
        assert_eq!(title, "Guide ready");
        assert!(body.contains("L4 Guide.md"), "{body}");

        let (title, body) = ending_message(Some("L4 Guide.md"), -1, false);
        assert_eq!(title, "Guide Watcher needs you");
        assert!(body.contains("L4 Guide.md"), "{body}");
        assert!(body.contains("see why"), "{body}");

        // A guide file that was never written is a failure even with a zero exit code.
        assert_eq!(ending_message(None, 0, false).0, "Guide Watcher needs you");
        assert_eq!(ending_message(None, 0, true).0, "Guide ready");

        let (_, one) = sources_message(1);
        assert!(one.contains("1 new lecture file is"), "{one}");
        let (_, many) = sources_message(4);
        assert!(many.contains("4 new lecture files are"), "{many}");
    }

    #[test]
    fn a_job_is_named_once_and_only_its_own_ending_consumes_the_name() {
        remember_guide("job-a", r"C:\Course\L4___Divide_and_Conquer_II_Guide.md");
        remember_guide("job-b", "/course/CH02_Guide.md");
        let taken = guide_names().lock().unwrap().remove("job-a");
        assert_eq!(taken.as_deref(), Some("L4___Divide_and_Conquer_II_Guide.md"));
        assert_eq!(
            guide_names().lock().unwrap().remove("job-b").as_deref(),
            Some("CH02_Guide.md")
        );
        assert!(guide_names().lock().unwrap().remove("job-a").is_none());
        assert_eq!(guide_display_name("Guide.md"), "Guide.md");
        assert_eq!(guide_display_name(r"C:\dir\"), "dir");
    }

    #[test]
    fn the_badge_is_a_red_disc_with_a_white_count_and_nothing_at_zero() {
        assert!(badge_icon(0).is_none());
        assert_eq!(badge_label(1), "1");
        assert_eq!(badge_label(9), "9");
        assert_eq!(badge_label(10), "9+");
        assert_eq!(badge_label(4_000), "9+");

        let mut ink_columns = std::collections::HashMap::new();
        for count in [1usize, 9, 12] {
            let badge = badge_icon(count).expect("a positive count has a badge");
            assert_eq!(badge.width(), BADGE_PX);
            assert_eq!(badge.height(), BADGE_PX);
            let rgba = badge.rgba();
            assert_eq!(rgba.len(), (BADGE_PX * BADGE_PX * 4) as usize);

            let pixels = rgba.chunks_exact(4);
            let ink = pixels
                .clone()
                .filter(|pixel| pixel[3] > 200 && pixel[0] > 230 && pixel[1] > 230 && pixel[2] > 230)
                .count();
            let fill = pixels
                .clone()
                .filter(|pixel| pixel[3] > 200 && pixel[0] > 150 && pixel[1] < 120)
                .count();
            let corners = [0usize, (BADGE_PX - 1) as usize];
            assert!(ink > 10, "count {count} drew no visible digits ({ink} px)");
            ink_columns.insert(count, ink_span(rgba));
            assert!(fill > 200, "count {count} drew no disc ({fill} px)");
            for y in corners {
                for x in corners {
                    let offset = (y * BADGE_PX as usize + x) * 4;
                    assert_eq!(rgba[offset + 3], 0, "the disc must not fill the corners");
                }
            }
        }
        // A tray-sized badge is drawn at its own size, not scaled up from the taskbar one.
        let large = badge_image(3, 96).expect("a positive count has a badge");
        assert_eq!((large.width(), large.height()), (96, 96));
        assert!(badge_image(3, 0).is_none());
        assert_eq!(tray_tooltip(0), "Guide Watcher");
        assert_eq!(tray_tooltip(1), "Guide Watcher - 1 update");
        assert_eq!(tray_tooltip(7), "Guide Watcher - 7 updates");

        // "9+" is two glyphs, so it must occupy more columns than one digit does; equal spans
        // would mean the pen never advanced and the glyphs were drawn on top of each other.
        assert!(
            ink_columns[&12] > ink_columns[&9],
            "two-character badge is not wider than one: {ink_columns:?}"
        );
    }

    /// How many pixel columns the white label covers.
    fn ink_span(rgba: &[u8]) -> usize {
        let side = BADGE_PX as usize;
        (0..side)
            .filter(|x| {
                (0..side).any(|y| {
                    let pixel = &rgba[(y * side + x) * 4..][..4];
                    pixel[3] > 200 && pixel[0] > 230 && pixel[1] > 230 && pixel[2] > 230
                })
            })
            .count()
    }
}
