//! Fit the initial native window to this monitor's usable desktop in physical pixels.
use tauri::{PhysicalPosition, PhysicalSize, WebviewWindow};

fn bounded_inner(inner: (u32, u32), outer: (u32, u32), work: (u32, u32)) -> (u32, u32) {
    let border = (
        outer.0.saturating_sub(inner.0),
        outer.1.saturating_sub(inner.1),
    );
    (
        inner.0.min(work.0.saturating_sub(border.0).max(1)),
        inner.1.min(work.1.saturating_sub(border.1).max(1)),
    )
}
pub(crate) fn fit(window: &WebviewWindow) -> tauri::Result<()> {
    let Some(monitor) = window.current_monitor()? else {
        return Ok(());
    };
    let work = monitor.work_area();
    if work.size.width == 0 || work.size.height == 0 {
        return Ok(());
    }
    let inner = window.inner_size()?;
    let outer = window.outer_size()?;
    let size = bounded_inner(
        (inner.width, inner.height),
        (outer.width, outer.height),
        (work.size.width, work.size.height),
    );
    let scale = monitor.scale_factor();
    window.set_min_size(Some(PhysicalSize::new(
        ((360.0 * scale) as u32).min(size.0),
        ((400.0 * scale) as u32).min(size.1),
    )))?;
    if size != (inner.width, inner.height) {
        window.set_size(PhysicalSize::new(size.0, size.1))?;
    }
    let outer = window.outer_size()?;
    let position = window.outer_position()?;
    let right = i64::from(work.position.x) + i64::from(work.size.width);
    let bottom = i64::from(work.position.y) + i64::from(work.size.height);
    if position.x < work.position.x
        || position.y < work.position.y
        || i64::from(position.x) + i64::from(outer.width) > right
        || i64::from(position.y) + i64::from(outer.height) > bottom
    {
        window.set_position(PhysicalPosition::new(
            work.position
                .x
                .saturating_add((work.size.width.saturating_sub(outer.width) / 2) as i32),
            work.position
                .y
                .saturating_add((work.size.height.saturating_sub(outer.height) / 2) as i32),
        ))?;
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn usable_area_includes_taskbar_dpi_and_native_chrome() {
        assert_eq!(
            bounded_inner((1280, 850), (1296, 889), (1920, 1040)),
            (1280, 850)
        );
        assert_eq!(
            bounded_inner((1920, 1275), (1944, 1334), (1366, 728)),
            (1342, 669)
        );
        assert_eq!(
            bounded_inner((2560, 1700), (2592, 1778), (1920, 1000)),
            (1888, 922)
        );
        assert_eq!(
            bounded_inner((360, 400), (376, 439), (320, 300)),
            (304, 261)
        );
        assert_eq!(bounded_inner((100, 100), (200, 200), (50, 50)), (1, 1));
    }
}
