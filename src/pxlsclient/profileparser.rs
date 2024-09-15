use anyhow::anyhow;
use regex::Regex;
use scraper::{ElementRef, Html, Selector};

use crate::pxlsclient::types::UserProfileBuilder;

use super::types::UserProfile;

const HTML_DISCORD_TAG: &str = "Discord Tag";
const DISCORD_TAG_NOT_SET: &str = "Not Set";
const HTML_FACTION: &str = "Faction";
const HTML_CANVAS_PIXEL: &str = "Canvas Pixels";
const HTML_CURRENT_CANVAS_PIXEL: &str = "Current Canvas Pixels";

/// Parses a profile from a HTML-encoded string
pub(super) trait ProfileParser {
    async fn get_profile_from_encoded_string(
        encoded_str: &str,
    ) -> Result<UserProfile, anyhow::Error>;
}

pub struct ProfileParserImpl {}

impl ProfileParserImpl {
    fn get_sibiling_td<'a>(element_ref: ElementRef<'a>) -> Option<ElementRef<'a>> {
        element_ref
            .parent()
            .and_then(|node| {
                node.children()
                    .filter(|node| {
                        matches!(
                            node.value()
                                .as_element()
                                .map(|element| element.name() == "td"),
                            Some(true)
                        )
                    })
                    .next()
            })
            .and_then(|node| ElementRef::wrap(node))
    }

    fn process_html_discord_tag(
        accum: UserProfileBuilder,
        element_ref: ElementRef,
    ) -> UserProfileBuilder {
        if let Some(builder) = Self::get_sibiling_td(element_ref)
            .map(|element| element.inner_html())
            .map(|candidate_tag| {
                if candidate_tag == DISCORD_TAG_NOT_SET {
                    accum.clone()
                } else {
                    accum.clone().discord_tag(candidate_tag)
                }
            })
        {
            builder
        } else {
            accum
        }
    }

    fn process_html_faction(
        accum: UserProfileBuilder,
        element_ref: ElementRef,
    ) -> UserProfileBuilder {
        if let Some(Ok(faction_id)) = Self::get_sibiling_td(element_ref)
            .map(|element| element.inner_html())
            .and_then(|value| {
                Regex::new(r#"(\d+)\)$"#)
                    .unwrap()
                    .captures(value.as_str())
                    .and_then(|captures| {
                        captures
                            .get(1)
                            .map(|id| u64::from_str_radix(id.as_str(), 10))
                    })
            })
        {
            accum.faction_id(faction_id)
        } else {
            accum
        }
    }

    fn process_html_canvas_pixel(
        accum: UserProfileBuilder,
        element_ref: ElementRef,
    ) -> UserProfileBuilder {
        if let Some(canvas_pixels) = Self::get_sibiling_td(element_ref)
            .map(|element| element.inner_html())
            .and_then(|value| u64::from_str_radix(&value.replace(",", ""), 10).ok())
        {
            accum.pixels(canvas_pixels)
        } else {
            accum
        }
    }
}

impl ProfileParser for ProfileParserImpl {
    async fn get_profile_from_encoded_string(
        encoded_str: &str,
    ) -> Result<UserProfile, anyhow::Error> {
        let document = Html::parse_document(encoded_str);
        let details_selector = Selector::parse("#tab-details").unwrap();
        let table_details_selector = Selector::parse("th").unwrap();

        document
            .select(&details_selector)
            .next()
            .ok_or(anyhow!("cannot find tab-details in the html"))?
            .select(&table_details_selector)
            .fold(
                UserProfileBuilder::default(),
                |accum, col_header| match col_header.inner_html().as_str() {
                    HTML_DISCORD_TAG => Self::process_html_discord_tag(accum, col_header),
                    HTML_FACTION => Self::process_html_faction(accum, col_header),
                    HTML_CANVAS_PIXEL => Self::process_html_canvas_pixel(accum, col_header),
                    HTML_CURRENT_CANVAS_PIXEL => Self::process_html_canvas_pixel(accum, col_header),
                    _ => accum,
                },
            )
            .build()
    }
}

#[cfg(test)]
mod tests {
    use crate::pxlsclient::profileparser::ProfileParserImpl;

    use super::ProfileParser;

    fn get_test_fixture(path: &str) -> String {
        std::fs::read_to_string(path)
            .expect("cannot read fixture. are you running from project root?")
    }

    #[tokio::test]
    async fn test_profile_from_html_document() {
        let fixture = get_test_fixture("./fixtures/sample_profile.html");
        let body = ProfileParserImpl::get_profile_from_encoded_string(&fixture)
            .await
            .expect("should be able to parse the fixture html");

        assert_eq!(body.discord_tag, Some("gbritannia".to_string()));
        assert_eq!(body.faction_id, Some(3680));
        assert_eq!(body.pixels, Some(6356));
    }

    #[tokio::test]
    async fn test_profile_from_another_html_document() {
        let fixture = get_test_fixture("./fixtures/sample_profile_2.html");
        let body = ProfileParserImpl::get_profile_from_encoded_string(&fixture)
            .await
            .expect("should be able to parse the fixture html");

        assert_eq!(body.discord_tag, None);
        assert_eq!(body.faction_id, Some(3680));
        assert_eq!(body.pixels, Some(3023));
    }

    #[tokio::test]
    async fn test_profile_from_ironmouse_document() {
        let fixture = get_test_fixture("./fixtures/sample_profile_3.html");
        let body = ProfileParserImpl::get_profile_from_encoded_string(&fixture)
            .await
            .expect("should be able to parse the fixture html");

        assert_eq!(body.discord_tag, None);
        assert_eq!(body.faction_id, Some(1));
        assert_eq!(body.pixels, Some(1356));
    }
}
