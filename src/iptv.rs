use crate::args::Args;
use anyhow::{anyhow, Result};
use des::{
    cipher::{block_padding::Pkcs7, BlockEncryptMut, KeyInit},
    TdesEde3,
};
#[cfg(not(any(target_os = "android", target_os = "fuchsia", target_os = "linux")))]
use local_ip_address::list_afinet_netifas;
use log::{debug, info};
use rand::Rng;
use regex_lite::Regex;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::task::JoinSet;

fn parse_channel_mapping(mapping_str: &str) -> HashMap<String, String> {
    let mut mapping = HashMap::new();
    for pair in mapping_str.split(',') {
        if let Some((from, to)) = pair.split_once('=') {
            mapping.insert(from.trim().to_string(), to.trim().to_string());
        }
    }
    mapping
}

#[allow(dead_code)]
fn get_mapped_channel_name(channel_name: &str, mapping: &HashMap<String, String>) -> String {
    mapping.get(channel_name).cloned().unwrap_or_else(|| channel_name.to_string())
}

fn categorize_channel(channel_name: &str) -> String {
    if channel_name.contains("超高清") || channel_name.contains("4K") {
        "超清频道".to_string()
    } else if channel_name.contains("高清") || channel_name.contains("超清") || channel_name.contains("卫视") {
        "高清频道".to_string()
    } else {
        "普通频道".to_string()
    }
}

fn get_client_with_if(#[allow(unused_variables)] if_name: Option<&str>) -> Result<Client> {
    let timeout = Duration::new(5, 0);
    #[allow(unused_mut)]
    let mut client = Client::builder().timeout(timeout).cookie_store(true);

    #[cfg(not(any(target_os = "android", target_os = "fuchsia", target_os = "linux")))]
    if let Some(i) = if_name {
        let network_interfaces = list_afinet_netifas()?;
        for (name, ip) in network_interfaces.iter() {
            debug!("{}: {}", name, ip);
            if name == i {
                client = client.local_address(ip.to_owned());
                break;
            }
        }
    }

    #[cfg(any(target_os = "android", target_os = "fuchsia", target_os = "linux"))]
    if let Some(i) = if_name {
        client = client.interface(i);
    }

    Ok(client.build()?)
}

async fn get_base_url(client: &Client, args: &Args) -> Result<String> {
    let user = args.user.as_str();

    let params = [("Action", "Login"), ("return_type", "1"), ("UserID", user)];

    let url = reqwest::Url::parse_with_params(
        "http://eds.iptv.gd.cn:8082/EDS/jsp/AuthenticationURL",
        params,
    )?;

    let response = client.get(url).send().await?.error_for_status()?;

    let epgurl = reqwest::Url::parse(response.json::<AuthJson>().await?.epgurl.as_str())?;
    let base_url = format!(
        "{}://{}:{}",
        epgurl.scheme(),
        epgurl.host_str().ok_or(anyhow!("no host"))?,
        epgurl.port_or_known_default().ok_or(anyhow!("no host"))?,
    );
    debug!("Got base_url {base_url}");
    Ok(base_url)
}

#[derive(Serialize, Deserialize, Clone)]
pub(crate) struct Program {
    pub(crate) start: i64,
    pub(crate) stop: i64,
    pub(crate) title: String,
    pub(crate) desc: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub(crate) struct Channel {
    pub(crate) id: u64,
    pub(crate) name: String,
    pub(crate) rtsp: String,
    pub(crate) igmp: Option<String>,
    pub(crate) epg: Vec<Program>,
    pub(crate) category: String,
}

#[derive(Deserialize)]
struct AuthJson {
    epgurl: String,
}

#[derive(Deserialize)]
struct TokenJson {
    #[serde(rename = "EncryToken")]
    encry_token: String,
}

#[derive(Deserialize)]
struct PlaybillList {
    #[serde(rename = "playbillLites")]
    list: Vec<Bill>,
}

#[derive(Deserialize)]
struct Bill {
    name: String,
    #[serde(rename = "startTime")]
    start_time: i64,
    #[serde(rename = "endTime")]
    end_time: i64,
}

pub(crate) async fn get_channels(
    args: &Args,
    need_epg: bool,
    scheme: &str,
    host: &str,
) -> Result<Vec<Channel>> {
    info!("Obtaining channels");

    let user = args.user.as_str();
    let passwd = args.passwd.as_str();
    let mac = args.mac.as_str();
    let imei = args.imei.as_str();
    let ip = args.address.as_str();

    let client = get_client_with_if(args.interface.as_deref())?;

    let base_url = get_base_url(&client, args).await?;

    let params = [
        ("response_type", "EncryToken"),
        ("client_id", "smcphone"),
        ("userid", user),
    ];
    let url = reqwest::Url::parse_with_params(
        format!("{base_url}/EPG/oauth/v2/authorize").as_str(),
        params,
    )?;
    let response = client.get(url).send().await?.error_for_status()?;

    let token = response.json::<TokenJson>().await?.encry_token;

    debug!("Got token {token}");

    let enc = ecb::Encryptor::<TdesEde3>::new_from_slice(
        format!("{:X}", md5::compute(passwd.as_bytes()))[0..24].as_bytes(),
    );
    let enc = match enc {
        Ok(enc) => Ok(enc),
        Err(e) => Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            format!("Encrpy error {e}"),
        )),
    }?;
    let data = format!(
        "{}${token}${user}${imei}${ip}${mac}$$CTC",
        rand::thread_rng().gen_range(0..10000000),
    );
    let auth = hex::encode_upper(enc.encrypt_padded_vec_mut::<Pkcs7>(data.as_bytes()));

    debug!("Got auth {auth}");

    let params = [
        ("client_id", "smcphone"),
        ("DeviceType", "deviceType"),
        ("UserID", user),
        ("DeviceVersion", "deviceVersion"),
        ("userdomain", "2"),
        ("datadomain", "3"),
        ("accountType", "1"),
        ("authinfo", auth.as_str()),
        ("grant_type", "EncryToken"),
    ];
    let url =
        reqwest::Url::parse_with_params(format!("{base_url}/EPG/oauth/v2/token").as_str(), params)?;
    let _response = client.get(url).send().await?.error_for_status()?;

    let url = reqwest::Url::parse(format!("{base_url}/EPG/jsp/getchannellistHWCTC.jsp").as_str())?;

    let response = client.get(url).send().await?.error_for_status()?;

    let res = response.text().await?;
    let re = Regex::new("Authentication.CTCSetConfig\\('Channel','(.+?)'\\)")?;
    let mut channels = re
        .captures_iter(&res)
        .map(|cap| cap[1].to_string())
        .map(|s| {
            s.split("\",")
                .map(|s| s.split("=\"").collect::<Vec<_>>())
                .filter_map(|s| {
                    s.first()
                        .map(|a| String::from(*a))
                        .and_then(|a| s.get(1).map(|b| String::from(*b)).map(|b| (a, b)))
                })
                .collect::<HashMap<_, _>>()
        })
        .collect::<Vec<_>>();

    let channels = channels
        .iter_mut()
        .filter_map(|m| {
            m.get("ChannelID")
                .and_then(|i| str::parse::<u64>(i).ok())
                .map(|i| (i, m))
        })
        .filter_map(|(i, m)| m.get("ChannelName").cloned().map(|n| (i, n, m)))
        .filter_map(|(i, n, m)| {
            m.get("ChannelURL")
                .and_then(|u| {
                    let rtsp = u.split('|').find(|u| u.starts_with("rtsp"));
                    let igmp = u.split('|').find(|u| u.starts_with("igmp"));
                    rtsp.map(|rtsp| (rtsp, igmp))
                })
                .map(|(rtsp, igmp)| {
                    (
                        if args.rtsp_proxy {
                            rtsp.replace("rtsp://", &format!("{}://{}/rtsp/", scheme, host))
                        } else {
                            rtsp.to_string()
                        }
                        .replace("zoneoffset=0", "zoneoffset=480"),
                        igmp.map(|igmp| {
                            if args.udp_proxy {
                                igmp.replace("igmp://", &format!("{}://{}/udp/", scheme, host))
                            } else {
                                igmp.to_string()
                            }
                        }),
                    )
                })
                .map(|u| (i, n, u))
        })
        .map(|(i, n, (rtsp, igmp))| Channel {
            id: i,
            name: n.to_owned(),
            category: categorize_channel(&n),
            rtsp,
            igmp,
            epg: vec![],
        })
        .collect::<Vec<_>>();

    info!("Got {} channel(s)", channels.len());

    if !need_epg {
        return Ok(channels);
    }

    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();

    let mut tasks = JoinSet::new();

    for channel in channels.into_iter() {
        let params = [
            ("channelId", format!("{}", channel.id)),
            ("begin", format!("{}", now - 86400000 * 2)),
            ("end", format!("{}", now + 86400000 * 5)),
        ];
        let url = reqwest::Url::parse_with_params(
            format!("{base_url}/EPG/jsp/iptvsnmv3/en/play/ajax/_ajax_getPlaybillList.jsp").as_str(),
            params,
        )?;
        let client = client.clone();
        tasks.spawn(async move { (client.get(url).send().await, channel) });
    }
    let mut channels = vec![];
    while let Some(Ok((Ok(res), mut channel))) = tasks.join_next().await {
        if let Ok(play_bill_list) = res.json::<PlaybillList>().await {
            for bill in play_bill_list.list.into_iter() {
                channel.epg.push(Program {
                    start: bill.start_time,
                    stop: bill.end_time,
                    title: bill.name.clone(),
                    desc: bill.name,
                })
            }
        }
        channels.push(channel);
    }

    Ok(channels)
}

pub(crate) async fn get_icon(args: &Args, id: &str) -> Result<Vec<u8>> {
    let client = get_client_with_if(args.interface.as_deref())?;
    let base_url = get_base_url(&client, args).await?;

    // 先尝试用原始 ID 获取图标
    let url = reqwest::Url::parse(&format!(
        "{base_url}/EPG/jsp/iptvsnmv3/en/list/images/channelIcon/{}.png",
        id
    ))?;

    let response = client.get(url).send().await;
    
    match response {
        Ok(resp) => {
            match resp.error_for_status() {
                Ok(resp) => Ok(resp.bytes().await?.to_vec()),
                Err(_) => {
                    // 如果获取失败，尝试使用映射
                    if let Some(mapping_str) = &args.channel_mapping {
                        let _mapping = parse_channel_mapping(mapping_str);
                        // 这里需要实现通过频道名查找对应ID的逻辑
                        // 暂时返回错误，后续可以完善
                        Err(anyhow!("Icon not found for id: {}", id))
                    } else {
                        Err(anyhow!("Icon not found for id: {}", id))
                    }
                }
            }
        }
        Err(e) => Err(anyhow!("Network error: {}", e))
    }
}
