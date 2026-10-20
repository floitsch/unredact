// Copyright (C) 2026 Toit contributors.
use unredact::{
    calibration::default_fonts,
    native::Font,
    pixelation::{self, Grid},
    raster::{GrayImage, Rect},
};

fn text_image() -> GrayImage {
    let font = Font::open(&default_fonts()[0]).unwrap();
    font.render(
        "AVATAR office typography",
        44.0,
        true,
        0.0,
        12.25,
        58.5,
        680,
        100,
    )
    .unwrap()
}
#[test]
fn learns_block_size_and_both_phases_from_pixels() {
    let original = text_image();
    for block in [3, 5, 8, 11] {
        for (px, py) in [(0, 0), (1, 2), (block - 1, block - 1)] {
            let image = original.mosaic(block, px, py).unwrap();
            let result = pixelation::analyze(&image, 2, 16, 3).unwrap();
            assert!(
                !result.hypotheses.is_empty(),
                "No grid for {block}/{px}/{py}"
            );
            assert_eq!(
                result.hypotheses[0].grid,
                Grid {
                    block,
                    phase_x: px,
                    phase_y: py
                },
                "{result:?}"
            );
        }
    }
}
#[test]
fn learns_grid_under_small_intensity_noise() {
    let original = text_image();
    let mut image = original.mosaic(7, 2, 5).unwrap();
    for (i, pixel) in image.pixels.iter_mut().enumerate() {
        let noise = ((i.wrapping_mul(1103515245).wrapping_add(12345) >> 8) % 7) as i32 - 3;
        *pixel = (*pixel as i32 + noise).clamp(0, 255) as u8;
    }
    let result = pixelation::analyze(&image, 2, 16, 3).unwrap();
    assert_eq!(
        result.hypotheses[0].grid,
        Grid {
            block: 7,
            phase_x: 2,
            phase_y: 5
        }
    );
    assert!(result.hypotheses[0].within_block_rmse > 0.5);
}
#[test]
fn rejects_solid_redaction_unpixelated_text_and_noise() {
    let blank = GrayImage::white(300, 100).unwrap();
    let mut mask = blank.clone();
    mask.fill(
        Rect {
            x: 33.0,
            y: 17.0,
            w: 180.0,
            h: 45.0,
        },
        0,
    )
    .unwrap();
    let mut noise = blank.clone();
    let mut rng = 123u64;
    for p in &mut noise.pixels {
        rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
        *p = (rng >> 32) as u8;
    }
    for (label, image) in [
        ("blank", blank),
        ("opaque mask", mask),
        ("noise", noise),
        ("original text", text_image()),
    ] {
        let result = pixelation::analyze(&image, 2, 24, 3).unwrap();
        assert_eq!(result.status, "no_supported_grid", "{label}: {result:?}");
    }
}
#[test]
fn crop_offset_changes_phase_and_preserves_interior_evidence() {
    let original = text_image();
    let full = original.mosaic(7, 1, 4).unwrap();
    let rect = Rect {
        x: 9.0,
        y: 8.0,
        w: 620.0,
        h: 80.0,
    };
    let crop = full.crop(rect).unwrap();
    let analysis = pixelation::analyze(&crop, 2, 16, 3).unwrap();
    let expected = Grid {
        block: 7,
        phase_x: (1 + 9) % 7,
        phase_y: (4 + 8) % 7,
    };
    assert_eq!(analysis.hypotheses[0].grid, expected);
    let simulated = original
        .crop(rect)
        .unwrap()
        .mosaic(7, expected.phase_x, expected.phase_y)
        .unwrap();
    assert_eq!(expected.interior_mse(&crop, &simulated).unwrap(), 0.0);
}
#[test]
fn a_single_edge_or_too_few_boundaries_cannot_establish_a_grid() {
    let mut image = GrayImage::white(200, 80).unwrap();
    image
        .fill(
            Rect {
                x: 40.0,
                y: 20.0,
                w: 160.0,
                h: 60.0,
            },
            0,
        )
        .unwrap();
    let report = pixelation::analyze(&image, 2, 32, 3).unwrap();
    assert!(report.hypotheses.is_empty());
}
