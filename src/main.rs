use anyhow::{bail, Context};
use base64::{engine::general_purpose, Engine as _};
use clap::{Arg, Command};
use regex::Regex;
use std::fs::File;

const P_DEVICE_SN: &str = r#"var cover_mid\s*=\s*"?([^;"]+)\s*"?;"#;
const P_CURRENT_POWER: &str = r#"var webdata_now_p\s*=\s*"?([^;"]+)\s*"?;"#;
const P_YIELD_TODAY: &str = r#"var webdata_today_e\s*=\s*"?([^;"]+)\s*"?;"#;
const P_TOTAL_YIELD: &str = r#"var webdata_total_e\s*=\s*"?([^;"]+)\s*"?;"#;

#[derive(Debug)]
pub struct StatusData {
    pub device_sn: String,
    pub current_power: f64,
    pub yield_today: f64,
    pub total_yield: f64,
}

#[derive(serde::Deserialize, PartialEq, Debug)]
pub struct Config {
    pub inverters: Vec<Inverter>,
    pub influxdbs: Vec<BackendInfluxDB>,
}

#[derive(serde::Deserialize, PartialEq, Debug)]
pub struct Inverter {
    #[serde(rename = "statusPageUrl")]
    status_page_url: String,
    user: String,
    password: String,
}

#[derive(serde::Deserialize, PartialEq, Debug)]
pub struct BackendInfluxDB {
    #[serde(rename = "influxUrl")]
    pub influx_url: String,
    pub bucket: String,
    pub org: String,
    pub token: String,
    pub measurement: String,
}

impl Config {
    pub fn load() -> anyhow::Result<Self> {
        let matches = Command::new("Solar Info Grabber")
            .arg(Arg::new("inverters").long("inverters").env("SG_INVERTERS"))
            .arg(Arg::new("influxdbs").env("SG_INFLUXDBS"))
            .get_matches();
        let inverters = matches.get_one::<String>("inverters");
        let influxdbs = matches.get_one::<String>("influxdbs");

        let result = match (inverters, influxdbs) {
            (Some(inverters), Some(influxdbs)) => Self {
                inverters: serde_json::from_str(inverters)
                    .with_context(|| "Expected JSON for 'inverters'")?,
                influxdbs: serde_json::from_str(influxdbs)
                    .with_context(|| "Expected JSON for 'influxdbs'")
                    .unwrap_or(vec![]),
            },
            (Some(_), None) | (None, Some(_)) => {
                bail!("Supply all arguments or none")
            }
            _ => {
                let path = format!("/etc/{}.conf", env!("CARGO_BIN_NAME"));
                serde_json::from_reader(
                    File::open(&path)
                        .with_context(|| format!("Failed to load config file: {}", path))?,
                )?
            }
        };
        if result.inverters.is_empty() {
            bail!("No inverters given");
        }
        if result.influxdbs.is_empty() {
            bail!("No publishers given, try 'influxdbs' (SG_INFLUXDBS)");
        }
        Ok(result)
    }
}

impl BackendInfluxDB {
    pub fn publish(
        &self,
        StatusData {
            device_sn,
            current_power,
            yield_today,
            total_yield,
        }: &StatusData,
    ) -> anyhow::Result<()> {
        // influxdb2 crate forces the whole tokio ecosystem, so we'll do it manually
        let mut write_url = url::Url::parse(&self.influx_url)?;
        write_url.set_path("api/v2/write");
        // Just to be safe, escape the device string
        let device_sn = escape_tag_value(device_sn);
        let measurement = &self.measurement;
        ureq::post(write_url.as_str())
            .query_pairs([("bucket", self.bucket.as_str()), ("org", self.org.as_str())])
            .set("Authorization", &format!("Token {}", self.token))
            .send_string(&format!("{measurement},device={device_sn} currentPower={current_power},yieldToday={yield_today},totalYield={total_yield}"))?;
        Ok(())
    }
}

pub fn escape_tag_value(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace(' ', "\\ ")
        .replace('=', "\\=")
        .replace(',', "\\,")
}

impl StatusData {
    pub fn from_status_page(html: &str) -> anyhow::Result<Self> {
        lazy_static::lazy_static! {
            static ref R_DEVICE_SN : Regex = Regex::new(P_DEVICE_SN).unwrap();
            static ref R_CURRENT_POWER : Regex = Regex::new(P_CURRENT_POWER).unwrap();
            static ref R_YIELD_TODAY : Regex = Regex::new(P_YIELD_TODAY).unwrap();
            static ref R_TOTAL_YIELD : Regex = Regex::new(P_TOTAL_YIELD).unwrap();
        }
        Ok(Self {
            device_sn: R_DEVICE_SN
                .captures(html)
                .with_context(|| "Could not parse device sn")?[1]
                .trim()
                .to_string(),
            current_power: R_CURRENT_POWER
                .captures(html)
                .with_context(|| "Could not parse current power")?[1]
                .to_string()
                .parse::<f64>()?,
            yield_today: R_YIELD_TODAY
                .captures(html)
                .with_context(|| "Could not parse yield today")?[1]
                .to_string()
                .parse::<f64>()?,
            total_yield: R_TOTAL_YIELD
                .captures(html)
                .with_context(|| "Could not parse total yield")?[1]
                .to_string()
                .parse::<f64>()?,
        })
    }
}

impl Inverter {
    pub fn request_status(&self) -> anyhow::Result<StatusData> {
        let token = format!("{}:{}", self.user, self.password);
        let status_html = ureq::get(&self.status_page_url)
            .set(
                "Authorization",
                &format!("Basic {}", general_purpose::STANDARD_NO_PAD.encode(token)),
            )
            .call()?
            .into_string()?;
        StatusData::from_status_page(&status_html)
    }
}

fn main() -> anyhow::Result<()> {
    let config = Config::load()?;
    for src in &config.inverters {
        match src.request_status() {
            Ok(status_data) => {
                for dst in &config.influxdbs {
                    if let Err(err) = dst.publish(&status_data) {
                        eprintln!("Failed to publish data to '{}': {err}", dst.influx_url);
                    }
                }
            }
            Err(err) => {
                eprintln!(
                    "Failed to receive data from '{}': {err}",
                    src.status_page_url
                );
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_status_parsing() {
        let status_data = StatusData::from_status_page(
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
        assert_eq!(status_data.device_sn, "238483342");
        assert_eq!(status_data.current_power, 998.0);
        assert_eq!(status_data.yield_today, 99.0);
        assert_eq!(status_data.total_yield, 1010.2);
    }

    #[test]
    fn test_env_config() {
        let result: Config = temp_env::with_vars(
            [
                (
                    "SG_INVERTERS",
                    Some(
                        r#"[{"statusPageUrl":"http://inverter","user":"user","password":"password"}]"#,
                    ),
                ),
                (
                    "SG_INFLUXDBS",
                    Some(
                        r#"[{"influxUrl":"http://influx", "bucket": "bucket", "org": "org", "token": "token","measurement":"measurement"}]"#,
                    ),
                ),
            ],
            || Config::load().unwrap(),
        );
        assert_eq!(
            result,
            Config {
                inverters: vec![Inverter {
                    status_page_url: "http://inverter".to_string(),
                    user: "user".to_string(),
                    password: "password".to_string(),
                }],
                influxdbs: vec![BackendInfluxDB {
                    influx_url: "http://influx".to_string(),
                    bucket: "bucket".to_string(),
                    org: "org".to_string(),
                    token: "token".to_string(),
                    measurement: "measurement".to_string()
                }]
            }
        );
    }
}
