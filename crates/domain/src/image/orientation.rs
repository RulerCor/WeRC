//! Only IFD0 Orientation is needed. Do not follow thumbnail/GPS/Exif sub-IFDs.
use super::ImageInfo;
use zeroize::Zeroizing;

pub(super) fn read(exif: Option<&[u8]>) -> Result<u16, ()> {
    let Some(data) = exif else { return Ok(1) };
    let little = match data.get(..2) {
        Some(b"II") => true,
        Some(b"MM") => false,
        _ => return Err(()),
    };
    let short = |at: usize| -> Result<u16, ()> {
        let bytes = data.get(at..at.checked_add(2).ok_or(())?).ok_or(())?;
        let bytes: [u8; 2] = bytes.try_into().map_err(|_| ())?;
        Ok(if little {
            u16::from_le_bytes(bytes)
        } else {
            u16::from_be_bytes(bytes)
        })
    };
    let long = |at: usize| -> Result<u32, ()> {
        let bytes = data.get(at..at.checked_add(4).ok_or(())?).ok_or(())?;
        let bytes: [u8; 4] = bytes.try_into().map_err(|_| ())?;
        Ok(if little {
            u32::from_le_bytes(bytes)
        } else {
            u32::from_be_bytes(bytes)
        })
    };
    if short(2)? != 42 {
        return Err(());
    }
    let at = long(4)? as usize;
    if at < 8 {
        return Err(());
    }
    let count = short(at)? as usize;
    let end = at.checked_add(2 + count * 12 + 4).ok_or(())?;
    if end > data.len() {
        return Err(());
    }
    let mut value = None;
    for index in 0..count {
        let entry = at + 2 + index * 12;
        if short(entry)? != 0x0112 {
            continue;
        }
        if value.is_some() || short(entry + 2)? != 3 || long(entry + 4)? != 1 {
            return Err(());
        }
        let orientation = short(entry + 8)?;
        if !(1..=8).contains(&orientation) {
            return Err(());
        }
        value = Some(orientation);
    }
    Ok(value.unwrap_or(1))
}

/// Input was already decoded and bounded. Physically orient pixels before
/// metadata removal so every preview and native consumer sees the same image.
pub(super) fn apply(
    info: ImageInfo,
    rgba: Zeroizing<Vec<u8>>,
    orientation: u16,
) -> (ImageInfo, Zeroizing<Vec<u8>>) {
    if orientation == 1 {
        return (info, rgba);
    }
    let (w, h) = (info.width as usize, info.height as usize);
    let (width, height) = if orientation >= 5 { (h, w) } else { (w, h) };
    let mut pixels = Zeroizing::new(vec![0; rgba.len()]);
    for y in 0..h {
        for x in 0..w {
            let (dx, dy) = match orientation {
                2 => (w - 1 - x, y),
                3 => (w - 1 - x, h - 1 - y),
                4 => (x, h - 1 - y),
                5 => (y, x),
                6 => (h - 1 - y, x),
                7 => (h - 1 - y, w - 1 - x),
                8 => (y, w - 1 - x),
                _ => unreachable!("orientation validated before decoding"),
            };
            pixels[(dy * width + dx) * 4..(dy * width + dx + 1) * 4]
                .copy_from_slice(&rgba[(y * w + x) * 4..(y * w + x + 1) * 4]);
        }
    }
    (
        ImageInfo {
            width: width as u32,
            height: height as u32,
        },
        pixels,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_eight_orientations_preserve_six_distinct_pixels() {
        // Source rows ABC / DEF. Expectations are written out independently.
        let orders = [
            "ABCDEF", "CBAFED", "FEDCBA", "DEFABC", "ADBECF", "DAEBFC", "FCEBDA", "CFBEAD",
        ];
        for (index, expected) in orders.iter().enumerate() {
            let raw: Vec<_> = b"ABCDEF".iter().flat_map(|&v| [v, 0, 0, 255]).collect();
            let (info, pixels) = apply(
                ImageInfo {
                    width: 3,
                    height: 2,
                },
                Zeroizing::new(raw),
                index as u16 + 1,
            );
            assert_eq!(
                (info.width, info.height),
                if index >= 4 { (2, 3) } else { (3, 2) }
            );
            assert_eq!(
                pixels
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|p| p[0])
                    .collect::<Vec<_>>(),
                expected.as_bytes()
            );
            assert!(pixels.as_chunks::<4>().0.iter().all(|p| p[3] == 255));
        }
    }
    #[test]
    fn tiff_byte_orders_bounds_and_ambiguous_orientation() {
        for little in [true, false] {
            for value in 1..=8u16 {
                let u16b = |n: u16| {
                    if little {
                        n.to_le_bytes()
                    } else {
                        n.to_be_bytes()
                    }
                };
                let u32b = |n: u32| {
                    if little {
                        n.to_le_bytes()
                    } else {
                        n.to_be_bytes()
                    }
                };
                let mut data = Vec::from(if little { *b"II" } else { *b"MM" });
                data.extend(u16b(42));
                data.extend(u32b(8));
                data.extend(u16b(1));
                data.extend(u16b(0x0112));
                data.extend(u16b(3));
                data.extend(u32b(1));
                data.extend(u16b(value));
                data.extend([0; 6]);
                assert_eq!(read(Some(&data)), Ok(value));
                for n in 0..data.len() {
                    assert!(read(Some(&data[..n])).is_err());
                }
                let mut bad = data.clone();
                bad[18..20].copy_from_slice(&u16b(9));
                assert!(read(Some(&bad)).is_err());
                let mut bad = data.clone();
                bad[14..18].copy_from_slice(&u32b(2));
                assert!(read(Some(&bad)).is_err());
                let mut bad = data.clone();
                bad[4..8].copy_from_slice(&u32b(u32::MAX));
                assert!(read(Some(&bad)).is_err());
                let entry = data[10..22].to_vec();
                data.splice(22..22, entry);
                data[8..10].copy_from_slice(&u16b(2));
                assert!(read(Some(&data)).is_err());
            }
        }
        assert_eq!(read(None), Ok(1));
        assert_eq!(
            read(Some(b"II\x2a\x00\x08\x00\x00\x00\x00\x00\x00\x00\x00\x00")),
            Ok(1)
        );
    }
}
