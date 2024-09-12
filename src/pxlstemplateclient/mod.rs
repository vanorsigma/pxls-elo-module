mod types;
use std::{collections::HashMap, io::Cursor};

use image::ImageReader;
pub use types::{PxlsTemplateParameters, FilledPixelCoordinatable};

pub trait PxlsTemplateClient {
    async fn get_template_parameters_from_url(
        &self,
        url: &str,
    ) -> Result<PxlsTemplateParameters, anyhow::Error>;
}

#[derive(Default)]
pub struct PxlsTemplateClientImpl {}

impl PxlsTemplateClientImpl {
    async fn get_image(url: &str) -> Result<image::RgbaImage, anyhow::Error> {
        let image = reqwest::Client::new().get(url).send().await?;

        Ok(ImageReader::new(Cursor::new(image.bytes().await?))
            .with_guessed_format()?
            .decode()?
            .into())
    }
}

impl PxlsTemplateClient for PxlsTemplateClientImpl {
    async fn get_template_parameters_from_url(
        &self,
        url: &str,
    ) -> Result<PxlsTemplateParameters, anyhow::Error> {
        let response = reqwest::Client::new().get(url).send().await?;

        let url = response.url();

        // the query params come after the #, so they are counted as fragments
        let queries = url
            .fragment()
            .ok_or(anyhow::anyhow!("can't get fragment"))?
            .split("&")
            .filter_map(|entry| {
                let mut split_entry = entry.split("=");
                Some((split_entry.next()?, split_entry.next()?))
            })
            .collect::<HashMap<_, _>>();
        println!("{:#?}", queries);

        let template_url = queries
            .get("template")
            .ok_or(anyhow::anyhow!("cannot get template from query parameter"))?
            .to_string();

        let image = Self::get_image(&template_url).await?;
        let square_size = image.width()
            / u32::from_str_radix(
                queries
                    .get("tw")
                    .ok_or(anyhow::anyhow!("cannot get tw parameter"))?,
                10,
            )?;

        Ok(PxlsTemplateParameters {
            template: image,
            origin_x: i64::from_str_radix(
                queries
                    .get("ox")
                    .ok_or(anyhow::anyhow!("cannot get origin_x parameter"))?,
                10,
            )?,
            origin_y: i64::from_str_radix(
                queries
                    .get("oy")
                    .ok_or(anyhow::anyhow!("cannot get origin_y parameter"))?,
                10,
            )?,
            square_size,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::PxlsTemplateClient;

    #[ignore = "actual live test"]
    #[tokio::test]
    async fn test_get_template_parameters_from_url() {
        env_logger::init();
        let client = super::PxlsTemplateClientImpl::default();
        let parameters = client
            .get_template_parameters_from_url("https://temp.osupxls.ovh/special/?name=SwarmXosu")
            .await
            .expect("can get parameters");

        println!("{:#?}", parameters.origin_x);
        println!("{:#?}", parameters.origin_y);
        println!("{:#?}", parameters.square_size);
    }
}
