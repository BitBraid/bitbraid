//! Avatars: an identicon, a 5×5 symmetric pattern made from the e-mail
//! address on this machine, with nothing sent anywhere; or, where the
//! user asked for pictures and the author has one, that picture
//! (`crate::avatars`).

use std::sync::Arc;

use gpui_kit::*;

use crate::zoom::px;

/// The avatar of the person with this e-mail address, `size` each way:
/// their picture when pictures are on and there is one, else the
/// identicon.
pub fn avatar(address: &str, size: Pixels, dark: bool, cx: &App) -> AnyElement {
    // What is drawn larger than a row is high gets the larger picture.
    match crate::avatars::picture(address, size > px(24.), cx) {
        Some(picture) => img(picture)
            .w(size)
            .h(size)
            .flex_none()
            .rounded_full()
            .into_any_element(),
        None => identicon(address, size, dark).into_any_element(),
    }
}

/// The picture at a provider's address (an account's, a repository
/// owner's), when pictures from providers are on and it is in; else the
/// pattern made from `key`, which names the same person (their login).
pub fn picture_at(
    address: Option<&str>,
    key: &str,
    size: Pixels,
    dark: bool,
    cx: &App,
) -> AnyElement {
    let large = size > px(24.);
    match address.and_then(|address| crate::avatars::picture_at(address, large, cx)) {
        Some(picture) => img(picture)
            .w(size)
            .h(size)
            .flex_none()
            .rounded_full()
            .into_any_element(),
        None => identicon(key, size, dark).into_any_element(),
    }
}

/// Paint a picture as a disc of `radius` around `centre`, inside an
/// existing canvas.
pub fn paint_picture(
    window: &mut Window,
    picture: &Arc<RenderImage>,
    centre: Point<Pixels>,
    radius: Pixels,
) {
    let bounds = Bounds::new(
        point(centre.x - radius, centre.y - radius),
        gpui_kit::size(radius * 2.0, radius * 2.0),
    );
    // A picture that cannot be drawn leaves the ring empty this frame.
    let _ = window.paint_image(
        bounds,
        bounds,
        Corners::all(radius),
        picture.clone(),
        0,
        false,
    );
}

fn hash(text: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Paint a 5×5 identicon at `origin` with `cell`-sized cells inside an
/// existing canvas.
pub fn paint_identicon(
    window: &mut Window,
    key: &str,
    origin: Point<Pixels>,
    cell: Pixels,
    dark: bool,
) {
    let h = hash(key);
    let hue = (h % 360) as f32 / 360.0;
    let color = if dark {
        hsla(hue, 0.55, 0.62, 1.0)
    } else {
        hsla(hue, 0.55, 0.45, 1.0)
    };
    let bits = h >> 9;
    for row in 0..5u32 {
        for col in 0..5u32 {
            let source_col = if col > 2 { 4 - col } else { col };
            let bit = row * 3 + source_col;
            if (bits >> bit) & 1 == 1 {
                let at = point(origin.x + cell * col as f32, origin.y + cell * row as f32);
                window.paint_quad(fill(Bounds::new(at, gpui_kit::size(cell, cell)), color));
            }
        }
    }
}

/// Render an identicon of `size` pixels for `key` (usually the email).
pub fn identicon(key: &str, size: Pixels, dark: bool) -> impl IntoElement {
    let h = hash(key);
    let hue = (h % 360) as f32 / 360.0;
    let color = if dark {
        hsla(hue, 0.55, 0.62, 1.0)
    } else {
        hsla(hue, 0.55, 0.45, 1.0)
    };
    let bits = h >> 9;
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let cell = bounds.size.width / 5.0;
            for row in 0..5u32 {
                for col in 0..5u32 {
                    // Mirror columns 3 and 4 onto 1 and 0.
                    let source_col = if col > 2 { 4 - col } else { col };
                    let bit = row * 3 + source_col;
                    if (bits >> bit) & 1 == 1 {
                        let origin = point(
                            bounds.origin.x + cell * col as f32,
                            bounds.origin.y + cell * row as f32,
                        );
                        window.paint_quad(fill(
                            Bounds::new(origin, gpui_kit::size(cell, cell)),
                            color,
                        ));
                    }
                }
            }
        },
    )
    .w(size)
    .h(size)
}
