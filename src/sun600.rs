use crate::PublishData;
use anyhow::{bail, Context};
use base64::{engine::general_purpose, Engine as _};
use regex::Regex;
use serde::Deserialize;
use std::borrow::Cow;

const P_DEVICE_SN: &str = r#"var cover_mid\s*=\s*"?([^;"]+)\s*"?;"#;
const P_CURRENT_POWER: &str = r#"var webdata_now_p\s*=\s*"?([^;"]+)\s*"?;"#;
const P_YIELD_TODAY: &str = r#"var webdata_today_e\s*=\s*"?([^;"]+)\s*"?;"#;
const P_TOTAL_YIELD: &str = r#"var webdata_total_e\s*=\s*"?([^;"]+)\s*"?;"#;

#[derive(Deserialize, PartialEq, Debug)]
pub struct Inverter {
    #[serde(rename = "statusPageUrl")]
    pub status_page_url: String,
    pub user: String,
    pub password: String,
    pub device_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_location: Option<String>,
}

impl Inverter {
    pub fn id(&self) -> Cow<str> {
        (&self.device_name).into()
    }

    pub fn poll_data(&mut self) -> anyhow::Result<PublishData> {
        let token = format!("{}:{}", self.user, self.password);
        let html = ureq::get(&self.status_page_url)
            .set(
                "Authorization",
                &format!("Basic {}", general_purpose::STANDARD_NO_PAD.encode(token)),
            )
            .call()?
            .into_string()?;
        self.parse_html(&html)
    }

    fn parse_html(&self, html: &str) -> anyhow::Result<PublishData> {
        lazy_static::lazy_static! {
            static ref R_DEVICE_SN : Regex = Regex::new(P_DEVICE_SN).unwrap();
            static ref R_CURRENT_POWER : Regex = Regex::new(P_CURRENT_POWER).unwrap();
            static ref R_YIELD_TODAY : Regex = Regex::new(P_YIELD_TODAY).unwrap();
            static ref R_TOTAL_YIELD : Regex = Regex::new(P_TOTAL_YIELD).unwrap();
        }
        let device_sn = R_DEVICE_SN
            .captures(html)
            .with_context(|| "Could not parse device sn")?[1]
            .trim()
            .to_string();
        let current_power = R_CURRENT_POWER
            .captures(html)
            .with_context(|| "Could not parse current power")?[1]
            .to_string()
            .parse::<f64>()?;
        let yield_today = R_YIELD_TODAY
            .captures(html)
            .with_context(|| "Could not parse yield today")?[1]
            .to_string()
            .parse::<f64>()?;
        let total_yield = R_TOTAL_YIELD
            .captures(html)
            .with_context(|| "Could not parse total yield")?[1]
            .to_string()
            .parse::<f64>()?;
        if current_power == 0.0 && yield_today == 0.0 && total_yield == 0.0 {
            bail!(
                "Filtering out device '{}' data (all values are zero).",
                device_sn
            )
        }
        let mut publisher = PublishData::default();
        publisher.tag("deviceName", self.device_name.clone());
        if let Some(device_location) = &self.device_location {
            publisher.tag("deviceLocation", device_location.clone());
        }
        publisher.tag("device", device_sn);
        publisher.field("currentPower", current_power);
        publisher.field("yieldToday", yield_today);
        publisher.field("totalYield", total_yield);
        Ok(publisher)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Value;

    #[test]
    fn test_status_parsing() {
        let status_data = Inverter {
            status_page_url: "some url".to_string(),
            device_location: Some("location".to_string()),
            device_name: "name".to_string(),
            password: "password".to_string(),
            user: "user".to_string(),
        }
        .parse_html(
            r#"
            var cover_mid = "238483342                             ";
var webdata_now_p = "998";
var webdata_today_e = "99.0";
var webdata_total_e = "1010.2";
var webdata_alarm = "";
var webdata_utime = "0";
        "#,
        )
        .unwrap();
        assert_eq!(
            status_data["device"],
            Value::String("238483342".to_string())
        );
        assert_eq!(status_data["currentPower"], Value::F64(998.0));
        assert_eq!(status_data["yieldToday"], Value::F64(99.0));
        assert_eq!(status_data["totalYield"], Value::F64(1010.2));
    }
}
