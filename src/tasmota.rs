use crate::PublishData;
use anyhow::Context;
use regex::Regex;
use std::borrow::Cow;
use std::net::Ipv4Addr;

#[derive(serde::Deserialize, PartialEq, Debug)]
pub struct Tasmota {
    ip: Ipv4Addr,
    pub device_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_location: Option<String>,
}

impl Tasmota {
    pub fn id(&self) -> Cow<str> {
        (&self.device_name).into()
    }

    pub fn poll_data(&mut self) -> anyhow::Result<PublishData> {
        let html = ureq::get(&format!("http://{}/?m=1", &self.ip))
            .call()?
            .into_string()?;
        self.parse_html(&html)
    }

    fn parse_html(&self, html: &str) -> anyhow::Result<PublishData> {
        lazy_static::lazy_static! {
            static ref R_CURRENT_POWER : Regex = Regex::new("Active Power[^>]*>[^>]*>([^<]*)").unwrap();
            static ref R_YIELD_TODAY : Regex = Regex::new("Energy Today[^>]*>[^>]*>([^<]*)").unwrap();
            static ref R_TOTAL_YIELD : Regex = Regex::new("Energy Total[^>]*>[^>]*>([^<]*)").unwrap();
        }
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
        let mut publisher = PublishData::default();
        publisher.tag("deviceName", self.device_name.clone());
        if let Some(device_location) = &self.device_location {
            publisher.tag("deviceLocation", device_location.clone());
        }
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
    fn function_name_test() {
        let data = r#"{t}</table><hr/>{t}{s}</th><th></th><th style='text-align:center'><th></th><td>{e}{s}Voltage{m}</td><td style='text-align:left'>234</td><td>&nbsp;</td><td> V{e}{s}Current{m}</td><td style='text-align:left'>0.000</td><td>&nbsp;</td><td> A{e}{s}Active Power{m}</td><td style='text-align:left'>344</td><td>&nbsp;</td><td> W{e}{s}Apparent Power{m}</td><td style='text-align:left'>0</td><td>&nbsp;</td><td> VA{e}{s}Reactive Power{m}</td><td style='text-align:left'>0</td><td>&nbsp;</td><td> VAr{e}{s}Power Factor{m}</td><td style='text-align:left'>0.00</td><td>&nbsp;</td><td>                         {e}{s}Energy Today{m}</td><td style='text-align:left'>0.289</td><td>&nbsp;</td><td> kWh{e}{s}Energy Yesterday{m}</td><td style='text-align:left'>0.002</td><td>&nbsp;</td><td> kWh{e}{s}Energy Total{m}</td><td style='text-align:left'>0.291</td><td>&nbsp;</td><td> kWh{e}</table><hr/>{t}</table>{t}<tr><td style='width:100%;text-align:center;font-weight:bold;font-size:62px'>ON</td></tr><tr></tr></table>"#;

        let status_data = Tasmota {
            device_location: Some("location".to_string()),
            device_name: "name".to_string(),
            ip: [127, 0, 0, 1].into(),
        }
        .parse_html(data)
        .unwrap();
        assert_eq!(status_data["currentPower"], Value::F64(344.0));
        assert_eq!(status_data["yieldToday"], Value::F64(0.289));
        assert_eq!(status_data["totalYield"], Value::F64(0.291));
    }
}
