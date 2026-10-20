//! The desktop binary's icon on Windows.
//!
//! Windows takes a program's icon from the program itself: the window
//! toolkit loads icon 1 of the executable for the window and the taskbar,
//! and Explorer shows the first icon there is. The icon is drawn here from
//! `brand/icon.svg` at the sizes Windows asks for, written into a
//! resource file and handed to the linker. Nothing is stored: the drawing
//! is the one source.
//!
//! Sources: Microsoft Learn, "Resource File Formats" (the RESOURCEHEADER
//! of a `.res` file), "Icons" and "The format of icon resources"
//! (GRPICONDIR, GRPICONDIRENTRY, PNG images in an icon); the
//! documentation of resvg 0.46.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_ENV").is_ok_and(|env| env == "msvc") {
        windows::icon();
    }
}

#[cfg(windows)]
mod windows {
    use std::path::{Path, PathBuf};

    /// The sizes of the icon, in pixels: those of the taskbar, the title
    /// bar, Explorer's views and the Start menu, at the usual scales.
    const SIZES: [u32; 9] = [16, 20, 24, 32, 40, 48, 64, 128, 256];

    /// Resource types.
    const RT_ICON: u16 = 3;
    const RT_GROUP_ICON: u16 = 14;

    /// Draw the icon and have the binary carry it. When it cannot be
    /// drawn the binary is built without it, and the build says why.
    pub fn icon() {
        let Some(manifest) = std::env::var_os("CARGO_MANIFEST_DIR") else {
            return;
        };
        let Some(out) = std::env::var_os("OUT_DIR").map(PathBuf::from) else {
            return;
        };
        let svg = Path::new(&manifest).join("../../brand/icon.svg");
        println!("cargo:rerun-if-changed={}", svg.display());
        let written = images(&svg).and_then(|images| {
            let res = out.join("bitbraid-icon.res");
            std::fs::write(&res, resources(&images)).map_err(|err| err.to_string())?;
            Ok(res)
        });
        match written {
            Ok(res) => println!("cargo:rustc-link-arg-bins={}", res.display()),
            Err(err) => println!("cargo:warning=the icon is left out: {err}"),
        }
    }

    /// The drawing at each size, as PNG.
    fn images(svg: &Path) -> Result<Vec<(u32, Vec<u8>)>, String> {
        use resvg::{tiny_skia, usvg};
        let data = std::fs::read(svg).map_err(|err| format!("{}: {err}", svg.display()))?;
        let tree = usvg::Tree::from_data(&data, &usvg::Options::default())
            .map_err(|err| format!("{}: {err}", svg.display()))?;
        let drawn = tree.size().width().max(tree.size().height());
        SIZES
            .iter()
            .map(|&size| {
                let mut pixmap =
                    tiny_skia::Pixmap::new(size, size).ok_or("no room for the picture")?;
                let scale = size as f32 / drawn;
                resvg::render(
                    &tree,
                    tiny_skia::Transform::from_scale(scale, scale),
                    &mut pixmap.as_mut(),
                );
                let png = pixmap.encode_png().map_err(|err| err.to_string())?;
                Ok((size, png))
            })
            .collect()
    }

    /// A resource file with one icon resource per image (ids 1 and up)
    /// and the group that lists them, icon 1.
    fn resources(images: &[(u32, Vec<u8>)]) -> Vec<u8> {
        let mut out = Vec::new();
        // A resource file begins with an empty entry.
        entry(&mut out, 0, 0, 0, &[]);
        let mut group = Vec::new();
        group.extend(0u16.to_le_bytes());
        group.extend(1u16.to_le_bytes());
        group.extend(u16::try_from(images.len()).unwrap_or(0).to_le_bytes());
        for (id, (size, png)) in (1u16..).zip(images) {
            entry(&mut out, RT_ICON, id, 0x1010, png);
            // 256 is written as 0: the field is one byte.
            let side = u8::try_from(*size).unwrap_or(0);
            group.extend([side, side, 0, 0]);
            group.extend(1u16.to_le_bytes());
            group.extend(32u16.to_le_bytes());
            group.extend(u32::try_from(png.len()).unwrap_or(0).to_le_bytes());
            group.extend(id.to_le_bytes());
        }
        entry(&mut out, RT_GROUP_ICON, 1, 0x1030, &group);
        out
    }

    /// One resource: a header naming its type and id by number, the data,
    /// and padding to four bytes.
    fn entry(out: &mut Vec<u8>, kind: u16, id: u16, flags: u16, data: &[u8]) {
        out.extend(u32::try_from(data.len()).unwrap_or(0).to_le_bytes());
        out.extend(32u32.to_le_bytes());
        out.extend([0xff, 0xff]);
        out.extend(kind.to_le_bytes());
        out.extend([0xff, 0xff]);
        out.extend(id.to_le_bytes());
        // Data version, memory flags, language (neutral), version,
        // characteristics.
        out.extend(0u32.to_le_bytes());
        out.extend(flags.to_le_bytes());
        out.extend(0u16.to_le_bytes());
        out.extend(0u32.to_le_bytes());
        out.extend(0u32.to_le_bytes());
        out.extend(data);
        while !out.len().is_multiple_of(4) {
            out.push(0);
        }
    }
}
