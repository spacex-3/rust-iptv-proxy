use actix_web::{
    get,
    web::{Data, Path, Query},
    App, HttpRequest, HttpResponse, HttpServer, Responder,
};
use anyhow::{anyhow, Result};
use argh::FromArgs;
use chrono::{FixedOffset, TimeZone, Utc};
use log::debug;
use reqwest::Client;
use std::{
    collections::{BTreeMap, HashMap},
    io::{BufWriter, Cursor, Read},
    net::SocketAddrV4,
    process::exit,
    str::FromStr,
    sync::Mutex,
};
use xml::{
    reader::XmlEvent as XmlReadEvent,
    writer::{EmitterConfig, XmlEvent as XmlWriteEvent},
    EventReader,
};

mod args;
use args::Args;

mod iptv;
use iptv::{get_channels, get_icon, Channel};

mod proxy;

static OLD_PLAYLIST: Mutex<Option<String>> = Mutex::new(None);
static OLD_XMLTV: Mutex<Option<String>> = Mutex::new(None);

fn parse_channel_mapping(mapping_str: &str) -> HashMap<String, String> {
    let mut mapping = HashMap::new();
    for pair in mapping_str.split(',') {
        if let Some((from, to)) = pair.split_once('=') {
            mapping.insert(from.trim().to_string(), to.trim().to_string());
        }
    }
    mapping
}

fn find_mapped_channel_id(channel_name: &str, channels: &[Channel], mapping: &HashMap<String, String>) -> u64 {
    // 先尝试映射
    if let Some(mapped_name) = mapping.get(channel_name) {
        // 查找映射后的频道名对应的ID
        for ch in channels {
            if ch.name == *mapped_name {
                return ch.id;
            }
        }
    }
    // 如果没有映射或者映射的频道不存在，返回0作为默认值
    0
}

fn to_xmltv_time(unix_time: i64) -> Result<String> {
    match Utc.timestamp_millis_opt(unix_time) {
        chrono::LocalResult::Single(t) => Ok(t
            .with_timezone(&FixedOffset::east_opt(8 * 60 * 60).ok_or(anyhow!(""))?)
            .format("%Y%m%d%H%M%S")
            .to_string()),
        _ => Err(anyhow!("fail to parse time")),
    }
}

fn to_xmltv<R: Read>(channels: Vec<Channel>, extra: Option<EventReader<R>>, mapping: &HashMap<String, String>) -> Result<String> {
    let mut buf = BufWriter::new(Vec::new());
    let mut writer = EmitterConfig::new()
        .perform_indent(false)
        .create_writer(&mut buf);
    writer.write(
        XmlWriteEvent::start_element("tv")
            .attr("generator-info-name", "iptv-proxy")
            .attr("source-info-name", "iptv-proxy"),
    )?;
    for channel in channels.iter() {
        writer.write(
            XmlWriteEvent::start_element("channel").attr("id", &format!("{}", channel.id)),
        )?;
        writer.write(XmlWriteEvent::start_element("display-name"))?;
        writer.write(XmlWriteEvent::characters(&channel.name))?;
        writer.write(XmlWriteEvent::end_element())?;
        writer.write(XmlWriteEvent::end_element())?;
    }
    if let Some(extra) = extra {
        for e in extra {
            match e {
                Ok(XmlReadEvent::StartElement {
                    name, attributes, ..
                }) => {
                    let name = name.to_string();
                    let name = name.as_str();
                    if name != "channel"
                        && name != "display-name"
                        && name != "desc"
                        && name != "title"
                        && name != "sub-title"
                        && name != "programme"
                    {
                        continue;
                    }
                    let name = if name == "title" {
                        let mut iter = attributes.iter();
                        loop {
                            let attr = iter.next();
                            if attr.is_none() {
                                break "title";
                            }
                            let attr = attr.unwrap();
                            if attr.name.to_string() == "lang" && attr.value != "chi" {
                                break "title_extra";
                            }
                        }
                    } else {
                        name
                    };
                    let mut tag = XmlWriteEvent::start_element(name);
                    for attr in attributes.iter() {
                        tag = tag.attr(attr.name.borrow(), &attr.value);
                    }
                    writer.write(tag)?;
                }
                Ok(XmlReadEvent::Characters(content)) => {
                    writer.write(XmlWriteEvent::characters(&content))?;
                }
                Ok(XmlReadEvent::EndElement { name }) => {
                    let name = name.to_string();
                    let name = name.as_str();
                    if name != "channel"
                        && name != "display-name"
                        && name != "desc"
                        && name != "title"
                        && name != "sub-title"
                        && name != "programme"
                    {
                        continue;
                    }
                    writer.write(XmlWriteEvent::end_element())?;
                }
                _ => {}
            }
        }
    }
    for channel in channels.iter() {
        for epg in channel.epg.iter() {
            writer.write(
                XmlWriteEvent::start_element("programme")
                    .attr("start", &format!("{} +0800", to_xmltv_time(epg.start)?))
                    .attr("stop", &format!("{} +0800", to_xmltv_time(epg.stop)?))
                    .attr("channel", &format!("{}", channel.id)),
            )?;
            writer.write(XmlWriteEvent::start_element("title").attr("lang", "chi"))?;
            writer.write(XmlWriteEvent::characters(&epg.title))?;
            writer.write(XmlWriteEvent::end_element())?;
            if !epg.desc.is_empty() {
                writer.write(XmlWriteEvent::start_element("desc"))?;
                writer.write(XmlWriteEvent::characters(&epg.desc))?;
                writer.write(XmlWriteEvent::end_element())?;
            }
            writer.write(XmlWriteEvent::end_element())?;
        }
        
        // 如果当前频道没有EPG数据，尝试使用映射频道的EPG数据
        if channel.epg.is_empty() {
            if let Some(mapped_name) = mapping.get(&channel.name) {
                // 查找映射的频道
                if let Some(mapped_channel) = channels.iter().find(|ch| ch.name == *mapped_name) {
                    for epg in mapped_channel.epg.iter() {
                        writer.write(
                            XmlWriteEvent::start_element("programme")
                                .attr("start", &format!("{} +0800", to_xmltv_time(epg.start)?))
                                .attr("stop", &format!("{} +0800", to_xmltv_time(epg.stop)?))
                                .attr("channel", &format!("{}", channel.id)), // 使用当前频道的ID
                        )?;
                        writer.write(XmlWriteEvent::start_element("title").attr("lang", "chi"))?;
                        writer.write(XmlWriteEvent::characters(&epg.title))?;
                        writer.write(XmlWriteEvent::end_element())?;
                        if !epg.desc.is_empty() {
                            writer.write(XmlWriteEvent::start_element("desc"))?;
                            writer.write(XmlWriteEvent::characters(&epg.desc))?;
                            writer.write(XmlWriteEvent::end_element())?;
                        }
                        writer.write(XmlWriteEvent::end_element())?;
                    }
                }
            }
        }
    }
    writer.write(XmlWriteEvent::end_element())?;
    Ok(String::from_utf8(buf.into_inner()?)?)
}

async fn parse_extra_xml(url: &str) -> Result<EventReader<Cursor<String>>> {
    let client = Client::builder().build()?;
    let url = reqwest::Url::parse(url)?;
    let response = client.get(url).send().await?.error_for_status()?;
    let xml = response.text().await?;
    let reader = Cursor::new(xml);
    Ok(EventReader::new(reader))
}

#[get("/xmltv")]
async fn xmltv(args: Data<Args>, req: HttpRequest) -> impl Responder {
    debug!("Get EPG");
    let scheme = req.connection_info().scheme().to_owned();
    let host = req.connection_info().host().to_owned();
    let extra_xml = match &args.extra_xmltv {
        Some(u) => parse_extra_xml(u).await.ok(),
        None => None,
    };
    
    let mapping = args.channel_mapping.as_ref()
        .map(|s| parse_channel_mapping(s))
        .unwrap_or_default();
        
    let xml = get_channels(&args, true, &scheme, &host)
        .await
        .and_then(|ch| to_xmltv(ch, extra_xml, &mapping));
    match xml {
        Err(e) => {
            if let Some(old_xmltv) = OLD_XMLTV.try_lock().ok().and_then(|f| f.to_owned()) {
                HttpResponse::Ok().content_type("text/xml").body(old_xmltv)
            } else {
                HttpResponse::InternalServerError().body(format!("Error getting channels: {}", e))
            }
        }
        Ok(xml) => HttpResponse::Ok().content_type("text/xml").body(xml),
    }
}

async fn parse_extra_playlist(url: &str) -> Result<String> {
    let client = Client::builder().build()?;
    let url = reqwest::Url::parse(url)?;
    let response = client.get(url).send().await?.error_for_status()?;
    Ok(response
        .text()
        .await?
        .strip_prefix("#EXTM3U")
        .map_or(String::from(""), |s| s.to_owned()))
}

#[get("/logo/{id}.png")]
async fn logo(args: Data<Args>, path: Path<String>) -> impl Responder {
    debug!("Get logo");
    match get_icon(&args, &path).await {
        Ok(icon) => HttpResponse::Ok().content_type("image/png").body(icon),
        Err(e) => HttpResponse::NotFound().body(format!("Error getting channels: {}", e)),
    }
}

#[get("/playlist")]
async fn playlist(args: Data<Args>, req: HttpRequest) -> impl Responder {
    debug!("Get playlist");
    let scheme = req.connection_info().scheme().to_owned();
    let host = req.connection_info().host().to_owned();
    match get_channels(&args, false, &scheme, &host).await {
        Err(e) => {
            if let Some(old_playlist) = OLD_PLAYLIST.try_lock().ok().and_then(|f| f.to_owned()) {
                HttpResponse::Ok()
                    .content_type("application/vnd.apple.mpegurl")
                    .body(old_playlist)
            } else {
                HttpResponse::InternalServerError().body(format!("Error getting channels: {}", e))
            }
        }
        Ok(ch) => {
            // 解析频道映射配置
            let mapping = args.channel_mapping.as_ref()
                .map(|s| parse_channel_mapping(s))
                .unwrap_or_default();
                
            let playlist = String::from("#EXTM3U\n")
                + &ch
                    .iter()
                    .map(|c| {
                        let group = if c.name.contains("超高清") || c.name.contains("4K") {
                            "超清频道"
                        } else if c.name.contains("高清") || c.name.contains("超清") || c.name.contains("卫视") {
                            "高清频道"
                        } else {
                            "普通频道"
                        };
                        let catch_up = format!(r#" catchup="append" catchup-source="{}?playseek=${{(b)yyyyMMddHHmmss}}-${{(e)yyyyMMddHHmmss}}" "#,
                            c.igmp.as_ref().map(|_| &c.rtsp).unwrap_or(&"".to_string()));
                        
                        // 查找映射的频道ID用于 logo 和 EPG
                        let mapped_id = find_mapped_channel_id(&c.name, &ch, &mapping);
                        let logo_id = if mapped_id != 0 { mapped_id } else { c.id };
                        
                        format!(
                            r#"#EXTINF:-1 tvg-id="{0}" tvg-name="{1}" tvg-chno="{0}"{3}tvg-logo="{4}://{5}/logo/{6}.png" group-title="{2}",{1}"#,
                            c.id, c.name, group, catch_up, scheme, host, logo_id
                        ) + "\n" + if args.udp_proxy { c.igmp.as_ref().unwrap_or(&c.rtsp) } else { &c.rtsp }
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
                + &match &args.extra_playlist {
                    Some(u) => parse_extra_playlist(u).await.unwrap_or(String::from("")),
                    None => String::from(""),
                };
            if let Ok(mut old_playlist) = OLD_PLAYLIST.try_lock() {
                *old_playlist = Some(playlist.clone());
            }
            HttpResponse::Ok()
                .content_type("application/vnd.apple.mpegurl")
                .body(playlist)
        }
    }
}

#[get("/rtsp/{tail:.*}")]
async fn rtsp(
    args: Data<Args>,
    mut path: Path<String>,
    mut params: Query<BTreeMap<String, String>>,
) -> impl Responder {
    let path = &mut *path;
    let params = &mut *params;
    let mut params = params.iter().map(|(k, v)| format!("{}={}", k, v));
    let param = params.next().unwrap_or("".to_string());
    let param = params.fold(param, |o, q| format!("{}&{}", o, q));
    HttpResponse::Ok().streaming(proxy::rtsp(
        format!("rtsp://{}?{}", path, param),
        args.interface.clone(),
    ))
}

#[get("/udp/{addr}")]
async fn udp(args: Data<Args>, addr: Path<String>) -> impl Responder {
    let addr = &*addr;
    let addr = match SocketAddrV4::from_str(addr) {
        Ok(addr) => addr,
        Err(e) => return HttpResponse::BadRequest().body(format!("Error: {}", e)),
    };
    HttpResponse::Ok().streaming(proxy::udp(addr, args.interface.clone()))
}

fn usage(cmd: &str) -> std::io::Result<()> {
    let usage = format!(
        r#"Usage: {} [OPTIONS] --user <USER> --passwd <PASSWD> --mac <MAC>

Options:
    -u, --user <USER>                      Login username
    -p, --passwd <PASSWD>                  Login password
    -m, --mac <MAC>                        MAC address
    -i, --imei <IMEI>                      IMEI [default: ]
    -b, --bind <BIND>                      Bind address:port [default: 0.0.0.0:7878]
    -a, --address <ADDRESS>                IP address/interface name [default: ]
    -I, --interface <INTERFACE>            Interface to request
        --extra-playlist <EXTRA_PLAYLIST>  Url to extra m3u
        --extra-xmltv <EXTRA_XMLTV>        Url to extra xmltv
        --channel-mapping <MAPPING>        Channel name mapping (format: "from1=to1,from2=to2")
        --udp-proxy                        Use UDP proxy
        --rtsp-proxy                       Use rtsp proxy
    -h, --help                             Print help
"#,
        cmd
    );
    eprint!("{}", usage);
    exit(0);
}

#[actix_web::main] // or #[tokio::main]
async fn main() -> std::io::Result<()> {
    env_logger::init();
    
    // 使用 argh 直接从环境解析参数
    let args: Args = argh::from_env();

    let bind_addr = args.bind.clone();
    
    HttpServer::new(move || {
        let args = Data::new(args.clone());
        App::new()
            .service(xmltv)
            .service(playlist)
            .service(logo)
            .service(rtsp)
            .service(udp)
            .app_data(args)
    })
    .bind(bind_addr)?
    .run()
    .await
}
