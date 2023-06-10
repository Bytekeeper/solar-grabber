mod sun600;
mod tasmota;

use crate::sun600::Inverter;
use crate::tasmota::Tasmota;
use anyhow::{bail, Context};
use clap::{Arg, Command};
use std::borrow::Cow;
use std::fs::File;

#[derive(serde::Deserialize, Debug, PartialEq)]
pub struct Config {
    pub sources: Vec<SourceDevice>,
    pub targets: Vec<BackendInfluxDB>,
}

#[derive(serde::Deserialize, Debug, PartialEq)]
pub struct BackendInfluxDB {
    #[serde(rename = "influxUrl")]
    pub influx_url: String,
    pub bucket: String,
    pub org: String,
    pub token: String,
    pub measurement: String,
}

#[derive(serde::Deserialize, Debug, PartialEq)]
#[serde(tag = "type")]
pub enum SourceDevice {
    Inverter(Inverter),
    Tasmota(Tasmota),
}

#[derive(Debug)]
pub enum Field {
    // Indexed
    Tag(String, Value),
    // Un-indexed
    Field(String, Value),
}

#[derive(Debug, PartialEq)]
pub enum Value {
    String(String),
    F64(f64),
}

#[derive(Default)]
pub struct PublishData {
    fields: Vec<Field>,
}

impl PublishData {
    pub fn tag(&mut self, name: impl Into<String>, value: impl Into<Value>) {
        self.fields.push(Field::Tag(name.into(), value.into()));
    }

    pub fn field(&mut self, name: impl Into<String>, value: impl Into<Value>) {
        self.fields.push(Field::Field(name.into(), value.into()));
    }
}

impl std::ops::Index<&str> for PublishData {
    type Output = Value;

    fn index(&self, index: &str) -> &Self::Output {
        self.fields
            .iter()
            .filter_map(|f| match f {
                Field::Tag(name, value) | Field::Field(name, value) if name == index => Some(value),
                _ => None,
            })
            .next()
            .unwrap()
    }
}

impl From<String> for Value {
    fn from(s: String) -> Self {
        Value::String(s)
    }
}

impl From<f64> for Value {
    fn from(f: f64) -> Self {
        Value::F64(f)
    }
}

impl SourceDevice {
    fn poll_data(&mut self) -> anyhow::Result<PublishData> {
        match self {
            SourceDevice::Inverter(d) => d.poll_data(),
            SourceDevice::Tasmota(d) => d.poll_data(),
        }
    }

    fn id(&self) -> Cow<str> {
        match self {
            SourceDevice::Inverter(d) => d.id(),
            SourceDevice::Tasmota(d) => d.id(),
        }
    }
}

impl Config {
    pub fn load() -> anyhow::Result<Self> {
        let matches = Command::new("Solar Info Grabber")
            .arg(Arg::new("sources").long("sources").env("SG_SOURCES"))
            .arg(Arg::new("targets").env("SG_INFLUXDBS"))
            .get_matches();
        let sources = matches.get_one::<String>("sources");
        let targets = matches.get_one::<String>("targets");

        let result = match (sources, targets) {
            (Some(sources), Some(targets)) => Self {
                sources: serde_json::from_str(sources)
                    .with_context(|| "Expected JSON for 'sources'")?,
                targets: serde_json::from_str(targets)
                    .with_context(|| "Expected JSON for 'targets'")
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
        if result.sources.is_empty() {
            bail!("No sources given");
        }
        if result.targets.is_empty() {
            bail!("No publishers given, try 'targets' (SG_INFLUXDBS)");
        }
        Ok(result)
    }
}

impl BackendInfluxDB {
    pub fn publish(&self, data: &PublishData) -> anyhow::Result<()> {
        // // influxdb2 crate forces the whole tokio ecosystem, so we'll do it manually
        let mut write_url = url::Url::parse(&self.influx_url)?;
        write_url.set_path("api/v2/write");
        let mut line = escape!(&self.measurement; ',' ' ');
        for f in &data.fields {
            if let Field::Tag(name, value) = f {
                line.push(',');
                line.push_str(&escape!(name; ',' '=' ' '));
                line.push('=');
                line.push_str(&match value {
                    Value::String(s) => escape!(s; ',' '=' ' '),
                    Value::F64(f) => f.to_string(),
                });
            }
        }
        line.push(' ');
        let mut first = true;
        for f in &data.fields {
            if let Field::Field(name, value) = f {
                if first {
                    first = false;
                } else {
                    line.push(',');
                }
                line.push_str(&escape!(name; ',' '=' ' '));
                line.push('=');
                line.push_str(&match value {
                    Value::String(s) => escape!(s; '"' '\\'),
                    Value::F64(f) => f.to_string(),
                });
            }
        }
        ureq::post(write_url.as_str())
            .query_pairs([("bucket", self.bucket.as_str()), ("org", self.org.as_str())])
            .set("Authorization", &format!("Token {}", self.token))
            .send_string(&line)?;
        Ok(())
    }
}

#[macro_export]
macro_rules! escape {
    ($i: expr ; $($l: literal)+) => {{
        let x = $i;
        let mut result = String::with_capacity(x.len());
        for c in x.chars() {
            match c {
                $(
                    $l => {
                        result.push('\\');
                    }
                )*
                _ => ()
            }
            result.push(c);
        }
        result
    }};
}

pub fn escape_tag_value(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace(' ', "\\ ")
        .replace('=', "\\=")
        .replace(',', "\\,")
}

fn main() -> anyhow::Result<()> {
    let mut config = Config::load()?;
    for src in &mut config.sources {
        match src.poll_data() {
            Ok(data) => {
                for dst in &config.targets {
                    if let Err(err) = dst.publish(&data) {
                        eprintln!("Failed to publish data to '{}': {err}", dst.influx_url);
                    }
                }
            }
            Err(err) => {
                eprintln!("Failed to receive data from '{}': {err}", src.id());
            }
        }
        // match src.request_status() {
        //     Ok(status_data) => {
        //     }
        //     Err(err) => {
        //     }
        // }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_env_config() {
        let result: Config = temp_env::with_vars(
            [
                (
                    "SG_SOURCES",
                    Some(
                        r#"[{"type":"Inverter","statusPageUrl":"http://inverter","user":"user","password":"password", "device_name":"the thing", "device_location":"backyard"}]"#,
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
                sources: vec![SourceDevice::Inverter(Inverter {
                    status_page_url: "http://inverter".to_string(),
                    user: "user".to_string(),
                    password: "password".to_string(),
                    device_name: "the thing".to_string(),
                    device_location: Some("backyard".to_string())
                })],
                targets: vec![BackendInfluxDB {
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
