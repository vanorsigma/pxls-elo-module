use image::Pixel;
use itertools::Itertools;

pub trait FilledPixelCoordinatable {
    fn get_filled_pixel_coordinates(&self) -> Vec<(u32, u32)>;
}

#[derive(Debug)]
pub struct PxlsTemplateParameters {
    pub template: image::RgbaImage,
    pub origin_x: i64,
    pub origin_y: i64,
    pub square_size: u32,
}

impl FilledPixelCoordinatable for PxlsTemplateParameters {
    fn get_filled_pixel_coordinates(&self) -> Vec<(u32, u32)> {
        (0..self.template.width() / self.square_size)
            .cartesian_product(0..self.template.height() / self.square_size)
            .map(|(square_x, square_y)| {
                (
                    square_x,
                    square_y,
                    (0..self.square_size)
                        .map(move |x| {
                            (0..self.square_size).map(move |y| {
                                self.template.get_pixel(
                                    square_x * self.square_size + x,
                                    square_y * self.square_size + y,
                                )
                            })
                        })
                        .flatten(),
                )
            })
            .filter_map(|(square_x, square_y, mut pixel_grid)| {
                if pixel_grid.any(|pixel| pixel.0[3] != 0) {
                    Some((square_x, square_y))
                } else {
                    None
                }
            })
            .map(|(square_x, square_y)| {
                (
                    square_x + self.origin_x as u32,
                    square_y + self.origin_y as u32,
                )
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use image::ImageReader;

    use super::{FilledPixelCoordinatable, PxlsTemplateParameters};

    #[test]
    fn test_sample_image() {
        let image = ImageReader::open("fixtures/sample_image.png")
            .expect("can open image")
            .decode()
            .expect("can decode image");

        let parameters = PxlsTemplateParameters {
            template: image.into(),
            origin_x: 1,
            origin_y: 0,
            square_size: 10,
        };

        let coordinates = parameters.get_filled_pixel_coordinates();
        println!("coordinates: {:#?}", coordinates);
        assert_eq!(coordinates.len(), 1);
        assert!(coordinates.contains(&(3, 3)))
    }

    #[test]
    fn test_sample_image_2() {
        let image = ImageReader::open("fixtures/sample_image_2.png")
            .expect("can open image")
            .decode()
            .expect("can decode image");

        let parameters = PxlsTemplateParameters {
            template: image.into(),
            origin_x: 1,
            origin_y: 0,
            square_size: 10,
        };

        let coordinates = parameters.get_filled_pixel_coordinates();
        println!("coordinates: {:#?}", coordinates);
        assert_eq!(coordinates.len(), 1);
        assert!(coordinates.contains(&(3, 3)))
    }
}
