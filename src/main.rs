use actix_web::{
    get, post,
    web::{Data, Path, Query, Json},
    App, HttpRequest, HttpResponse, HttpServer, Responder,
};
use actix_files as fs;
use anyhow::{anyhow, Result};
use chrono::{FixedOffset, TimeZone, Utc};
use log::{debug, info, warn, error};
use serde::Deserialize;
use reqwest::Client;
use std::{
    collections::{BTreeMap, HashMap},
    fs::{File, OpenOptions},
    io::{BufWriter, Cursor, Read, Write},
    net::SocketAddrV4,
    path::Path as StdPath,
    process::exit,
    str::FromStr,
    sync::{Mutex, LazyLock},
    time::{SystemTime, UNIX_EPOCH},
};
use xml::{
    reader::XmlEvent as XmlReadEvent,
    writer::{EmitterConfig, XmlEvent as XmlWriteEvent},
    EventReader,
};

mod args;
use args::Args;

mod iptv;
use iptv::{get_channels, get_icon, get_base_url, get_client_with_if, Channel, Program};

mod proxy;

static OLD_PLAYLIST: Mutex<Option<String>> = Mutex::new(None);
static OLD_XMLTV: Mutex<Option<String>> = Mutex::new(None);
static CHANNEL_MAPPINGS: LazyLock<Mutex<HashMap<u64, u64>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
static MAPPED_XMLTV_CACHE: Mutex<Option<String>> = Mutex::new(None);
static ALL_CHANNELS_EPG: LazyLock<Mutex<Option<Vec<Channel>>>> = LazyLock::new(|| Mutex::new(None));
const MAPPINGS_FILE: &str = "channel_mappings.json";
const XMLTV_CACHE_FILE: &str = "xmltv_cache.xml";
const EPG_CACHE_FILE: &str = "epg_cache.json";

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

// 加载频道映射从文件
fn load_mappings_from_file() -> Result<HashMap<u64, u64>> {
    if StdPath::new(MAPPINGS_FILE).exists() {
        let file = File::open(MAPPINGS_FILE)?;
        let mappings: HashMap<u64, u64> = serde_json::from_reader(file)?;
        Ok(mappings)
    } else {
        Ok(HashMap::new())
    }
}

// 保存频道映射到文件
fn save_mappings_to_file(mappings: &HashMap<u64, u64>) -> Result<()> {
    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(MAPPINGS_FILE)?;
    serde_json::to_writer_pretty(file, mappings)?;
    Ok(())
}

// 加载缓存的XMLTV
fn load_xmltv_cache() -> Result<Option<String>> {
    if StdPath::new(XMLTV_CACHE_FILE).exists() {
        let mut file = File::open(XMLTV_CACHE_FILE)?;
        let mut contents = String::new();
        file.read_to_string(&mut contents)?;
        Ok(Some(contents))
    } else {
        Ok(None)
    }
}

// 保存XMLTV缓存
fn save_xmltv_cache(xmltv_content: &str) -> Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(XMLTV_CACHE_FILE)?;
    file.write_all(xmltv_content.as_bytes())?;
    Ok(())
}

// 保存EPG缓存
fn save_epg_cache(channels: &[Channel]) -> Result<()> {
    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(EPG_CACHE_FILE)?;
    serde_json::to_writer(file, channels)?;
    Ok(())
}

// 加载EPG缓存
fn load_epg_cache() -> Result<Option<Vec<Channel>>> {
    if StdPath::new(EPG_CACHE_FILE).exists() {
        let file = File::open(EPG_CACHE_FILE)?;
        let channels: Vec<Channel> = serde_json::from_reader(file)?;
        Ok(Some(channels))
    } else {
        Ok(None)
    }
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
    // 为每个频道生成节目信息
    for channel in channels.iter() {
        let mut mapped_epg_used = false;
        let mut mapped_from = String::new();
        
        // 首先检查是否有映射
        if let Ok(mappings) = CHANNEL_MAPPINGS.try_lock() {
            log::debug!("Checking mappings for channel '{}' ({}): found {} mappings", 
                channel.name, channel.id, mappings.len());
            for (&from_id, &to_id) in mappings.iter() {
                log::debug!("  Mapping: {} -> {}", from_id, to_id);
            }
            
            // 查找是否有其他频道映射到当前频道
            let mut source_channels = Vec::new();
            for (&from_id, &to_id) in mappings.iter() {
                if to_id == channel.id {
                    source_channels.push(from_id);
                }
            }
            
            if !source_channels.is_empty() {
                // 当前频道是映射的目标，使用自己的EPG数据
                log::debug!("Channel '{}' ({}) is target for sources: {:?}", channel.name, channel.id, source_channels);
                // 使用自己的EPG数据（因为它是目标频道）
                if !channel.epg.is_empty() {
                    mapped_from = format!("target for channels {:?}", source_channels);
                    log::debug!("Channel '{}' ({}) using its own EPG ({} programs) as mapping target", 
                        channel.name, channel.id, channel.epg.len());
                    
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
                    mapped_epg_used = true;
                }
            } else if let Some(&mapped_id) = mappings.get(&channel.id) {
                // 当前频道是映射的源，使用目标频道的EPG数据
                log::debug!("Channel '{}' ({}) is source, mapping to target {}", channel.name, channel.id, mapped_id);
                if let Some(mapped_channel) = channels.iter().find(|ch| ch.id == mapped_id) {
                    if !mapped_channel.epg.is_empty() {
                        // 使用映射频道的EPG数据
                        mapped_from = format!("ID {} ({})", mapped_id, mapped_channel.name);
                        log::debug!("Channel '{}' ({}) using EPG from mapped channel '{}' ({} programs)", 
                            channel.name, channel.id, mapped_channel.name, mapped_channel.epg.len());
                        
                        for epg in mapped_channel.epg.iter() {
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
                        mapped_epg_used = true;
                    }
                }
            }
        }
        
        // 如果没有前端映射，尝试传统的名称映射
        if !mapped_epg_used {
            if let Some(mapped_name) = mapping.get(&channel.name) {
                if let Some(mapped_channel) = channels.iter().find(|ch| ch.name == *mapped_name) {
                    if !mapped_channel.epg.is_empty() {
                        // 使用映射频道的EPG数据
                        mapped_from = format!("name '{}' ({})", mapped_name, mapped_channel.name);
                        log::debug!("Channel '{}' ({}) using EPG from mapped channel '{}' ({} programs)", 
                            channel.name, channel.id, mapped_channel.name, mapped_channel.epg.len());
                        
                        for epg in mapped_channel.epg.iter() {
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
                        mapped_epg_used = true;
                    }
                }
            }
        }
        
        // 如果没有使用映射的EPG，使用自己的EPG数据
        if !mapped_epg_used {
            if !channel.epg.is_empty() {
                // 使用自己的EPG数据
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
            } else {
                log::warn!("Channel '{}' ({}) has no EPG and no mapping found", channel.name, channel.id);
            }
        } else {
            // 记录映射使用情况
            log::info!("Channel '{}' ({}) mapped from {}", channel.name, channel.id, mapped_from);
        }
    }
    writer.write(XmlWriteEvent::end_element())?;
    
    // 统计信息
    let total_channels = channels.len();
    let channels_with_epg = channels.iter().filter(|ch| !ch.epg.is_empty()).count();
    let channels_without_epg = total_channels - channels_with_epg;
    
    log::info!("XMLTV generation completed: {} total channels, {} with EPG, {} without EPG", 
        total_channels, channels_with_epg, channels_without_epg);
    
    Ok(String::from_utf8(buf.into_inner()?)?)
}

// 获取带EPG的频道列表（优先使用缓存）
async fn get_channels_with_epg(args: &Args, scheme: &str, host: &str) -> Result<Vec<Channel>> {
    // 首先尝试从缓存获取
    if let Ok(cache) = ALL_CHANNELS_EPG.try_lock() {
        if let Some(ref cached_channels) = *cache {
            log::info!("Using cached EPG data for {} channels", cached_channels.len());
            return Ok(cached_channels.clone());
        }
    }
    
    // 如果没有缓存，实时获取
    log::info!("No EPG cache found, fetching in real-time");
    get_channels(args, true, scheme, host).await
}

async fn parse_extra_xml(url: &str) -> Result<EventReader<Cursor<String>>> {
    let client = Client::builder().build()?;
    let url = reqwest::Url::parse(url)?;
    let response = client.get(url).send().await?.error_for_status()?;
    let xml = response.text().await?;
    let reader = Cursor::new(xml);
    Ok(EventReader::new(reader))
}

// 获取所有频道的EPG数据（带进度显示）
async fn fetch_all_channels_epg_simple(args: &Args) -> Result<Vec<Channel>> {
    log::info!("Starting EPG fetch for all channels");
    
    // 设置进度状态
    if let Ok(mut progress) = EPG_FETCH_PROGRESS.try_lock() {
        progress.is_fetching = true;
        progress.current = 0;
        progress.total = 0;
        progress.current_channel = String::new();
    }
    
    let scheme = "http";
    let host = &args.bind;
    
    // 先获取频道列表（这会进行认证并获取EPG数据）
    let channels = get_channels(args, true, scheme, host).await?;
    
    let channel_count = channels.len();
    log::info!("Got {} channels with EPG data", channel_count);
    
    // 更新总频道数
    if let Ok(mut progress) = EPG_FETCH_PROGRESS.try_lock() {
        progress.total = channel_count;
    }
    
    let _now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    let mut with_epg_count = 0;
    
    // 统计有EPG数据的频道
    for channel in channels.iter() {
        if !channel.epg.is_empty() {
            with_epg_count += 1;
        }
    }
    
    log::info!("EPG fetch completed: {}/{} channels have EPG data", with_epg_count, channel_count);
    
    // 重置进度状态
    if let Ok(mut progress) = EPG_FETCH_PROGRESS.try_lock() {
        progress.is_fetching = false;
        progress.current = channel_count;
        progress.current_channel = String::new();
    }
    
    Ok(channels)
}

// 获取所有频道的EPG数据
async fn fetch_all_channels_epg(args: &Args) -> Result<Vec<Channel>> {
    log::info!("Starting full EPG fetch for all channels");
    
    let scheme = "http";
    let host = &args.bind;
    
    // 先获取频道列表（这会进行认证）
    let channels = get_channels(args, false, scheme, host).await?;
    
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    
    let mut channels_with_epg = Vec::new();
    
    log::info!("Fetching EPG for {} channels", channels.len());
    
    // 限制并发数为1，避免被服务器拒绝
    let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(1));
    let mut handles = Vec::new();
    
    // 为每个频道创建一个已认证的client
    for channel in channels {
        let params = [
            ("channelId", format!("{}", channel.id)),
            ("begin", format!("{}", now - 86400000 * 2)), // 2天前
            ("end", format!("{}", now + 86400000 * 5)),   // 5天后
        ];
        
        // 为每个频道创建新的client和认证
        let client = get_client_with_if(args.interface.as_deref())?;
        let base_url = get_base_url(&client, args).await?;
        let url = reqwest::Url::parse_with_params(
            format!("{}/EPG/jsp/iptvsnmv3/en/play/ajax/_ajax_getPlaybillList.jsp", base_url).as_str(),
            params,
        )?;
        let permit = semaphore.clone().acquire_owned().await?;
        
        let handle = tokio::spawn(async move {
            let _permit = permit; // 持有permit直到任务完成
            let response = client.get(url).send().await;
            (response, channel)
        });
        handles.push(handle);
    }
    
    for handle in handles {
        if let Ok((Ok(res), mut channel)) = handle.await {
            log::debug!("Processing response for channel: {}", channel.name);
            
            let response_text = res.text().await.unwrap_or_else(|_| "Failed to get response text".to_string());
            
            match serde_json::from_str::<PlaybillList>(&response_text) {
                Ok(play_bill_list) => {
                    log::debug!("Got {} programs for channel '{}'", play_bill_list.list.len(), channel.name);
                    for bill in play_bill_list.list.into_iter() {
                        channel.epg.push(Program {
                            start: bill.start_time,
                            stop: bill.end_time,
                            title: bill.name.clone(),
                            desc: bill.name,
                        })
                    }
                    log::debug!("Fetched {} programs for channel '{}'", channel.epg.len(), channel.name);
                }
                Err(e) => {
                    log::warn!("Failed to parse EPG data for channel '{}': {}", channel.name, e);
                    log::debug!("Response for channel {}: {}", channel.name, response_text);
                }
            }
            channels_with_epg.push(channel);
        }
    }
    
    
    let with_epg = channels_with_epg.iter().filter(|ch| !ch.epg.is_empty()).count();
    let without_epg = channels_with_epg.len() - with_epg;
    
    log::info!("EPG fetch completed: {} channels with EPG, {} without EPG", with_epg, without_epg);
    
    Ok(channels_with_epg)
}

// 定时获取所有EPG数据
async fn fetch_all_epg_periodically(args: Data<Args>) {
    loop {
        log::info!("Starting scheduled EPG fetch...");
        
        // 使用简化版本获取EPG
        match fetch_all_channels_epg_simple(&args).await {
            Ok(channels) => {
                // 保存到缓存
                if let Ok(mut cache) = ALL_CHANNELS_EPG.try_lock() {
                    *cache = Some(channels.clone());
                    
                    // 保存到文件
                    if let Err(e) = save_epg_cache(&channels) {
                        log::error!("Failed to save EPG cache: {}", e);
                    } else {
                        log::info!("EPG cache updated and saved");
                    }
                }
                
                // 重新生成XMLTV
                let mapping = args.channel_mapping.as_ref()
                    .map(|s| parse_channel_mapping(s))
                    .unwrap_or_default();
                
                match to_xmltv(channels.clone(), None::<EventReader<Cursor<String>>>, &mapping) {
                    Ok(xmltv) => {
                        if let Ok(mut cache) = MAPPED_XMLTV_CACHE.try_lock() {
                            *cache = Some(xmltv.clone());
                            
                            if let Err(e) = save_xmltv_cache(&xmltv) {
                                log::error!("Failed to save XMLTV cache: {}", e);
                            } else {
                                log::info!("XMLTV regenerated after EPG fetch");
                            }
                        }
                    }
                    Err(e) => {
                        log::error!("Failed to regenerate XMLTV: {}", e);
                    }
                }
            }
            Err(e) => {
                log::error!("Failed to fetch EPG data: {}", e);
            }
        }
        
        // 每6小时执行一次
        tokio::time::sleep(tokio::time::Duration::from_secs(6 * 3600)).await;
    }
}

// 定时生成映射后的XMLTV
async fn generate_mapped_xmltv_periodically(args: Data<Args>) {
    let scheme = "http"; // 默认scheme
    let host = &args.bind;
    
    loop {
        tokio::time::sleep(tokio::time::Duration::from_secs(21600)).await; // 每6小时更新一次
        
        log::info!("Generating mapped XMLTV cache...");
        
        // 先输出当前的映射信息
        if let Ok(mappings) = CHANNEL_MAPPINGS.try_lock() {
            log::info!("Current channel mappings: {} mappings configured", mappings.len());
            for (&from_id, &to_id) in mappings.iter() {
                log::info!("  Mapping: {} -> {}", from_id, to_id);
            }
        }
        
        let extra_xml = match &args.extra_xmltv {
            Some(u) => parse_extra_xml(u).await.ok(),
            None => None,
        };
        
        let mapping = args.channel_mapping.as_ref()
            .map(|s| parse_channel_mapping(s))
            .unwrap_or_default();
            
        match get_channels(&args, true, scheme, host).await {
            Ok(channels) => {
                // 使用内存中的CHANNEL_MAPPINGS来生成XMLTV
                match to_xmltv_with_mappings(channels, extra_xml, &mapping).await {
                    Ok(xmltv) => {
                        if let Ok(mut cache) = MAPPED_XMLTV_CACHE.try_lock() {
                            *cache = Some(xmltv.clone());
                            
                            // 保存到文件
                            if let Err(e) = save_xmltv_cache(&xmltv) {
                                log::error!("Failed to save XMLTV cache: {}", e);
                            } else {
                                log::info!("XMLTV cache updated and saved");
                            }
                        }
                    }
                    Err(e) => {
                        log::error!("Failed to generate XMLTV: {}", e);
                    }
                }
            }
            Err(e) => {
                log::error!("Failed to get channels for XMLTV generation: {}", e);
            }
        }
    }
}

// 使用前端映射生成XMLTV
async fn to_xmltv_with_mappings<R: Read>(
    channels: Vec<Channel>,
    extra: Option<EventReader<R>>,
    mapping: &HashMap<String, String>,
) -> Result<String> {
    // 直接使用原始的to_xmltv函数，它已经包含了映射逻辑
    to_xmltv(channels, extra, mapping)
}

#[derive(serde::Deserialize, serde::Serialize)]
struct ChannelMapping {
    from_id: u64,
    to_id: u64,
}

#[derive(serde::Deserialize)]
struct MappingRequest {
    mappings: Vec<ChannelMapping>,
}

#[post("/api/channel-mappings")]
async fn api_set_channel_mappings(req: Json<MappingRequest>) -> impl Responder {
    debug!("Setting channel mappings");
    
    if let Ok(mut mappings) = CHANNEL_MAPPINGS.try_lock() {
        mappings.clear();
        for mapping in &req.mappings {
            mappings.insert(mapping.from_id, mapping.to_id);
        }
        
        // 保存到文件
        if let Err(e) = save_mappings_to_file(&mappings) {
            log::error!("Failed to save mappings to file: {}", e);
        }
        
        // 清除XMLTV缓存，强制重新生成
        if let Ok(mut cache) = MAPPED_XMLTV_CACHE.try_lock() {
            *cache = None;
        }
        
        // 清除EPG缓存，强制重新获取频道数据
        if let Ok(mut cache) = ALL_CHANNELS_EPG.try_lock() {
            *cache = None;
            log::info!("EPG cache cleared due to mapping changes");
        }
        
        HttpResponse::Ok().json("Mappings updated successfully")
    } else {
        HttpResponse::InternalServerError().json("Failed to update mappings")
    }
}

#[get("/api/cache-status")]
async fn api_cache_status() -> impl Responder {
    let cache_status = if let Ok(cache) = MAPPED_XMLTV_CACHE.try_lock() {
        match *cache {
            Some(_) => {
                if let Ok(Some(_file_cache)) = load_xmltv_cache() {
                    "cached_in_memory_and_file"
                } else {
                    "cached_in_memory_only"
                }
            }
            None => {
                if let Ok(Some(_)) = load_xmltv_cache() {
                    "cached_in_file_only"
                } else {
                    "not_cached"
                }
            }
        }
    } else {
        "cache_lock_error"
    };
    
    HttpResponse::Ok().json(cache_status)
}

// 全局EPG获取进度状态
static EPG_FETCH_PROGRESS: LazyLock<Mutex<EPGProgress>> = LazyLock::new(|| {
    Mutex::new(EPGProgress {
        is_fetching: false,
        current: 0,
        total: 0,
        current_channel: String::new(),
    })
});

#[derive(serde::Serialize, serde::Deserialize)]
struct EPGProgress {
    is_fetching: bool,
    current: usize,
    total: usize,
    current_channel: String,
}

#[get("/api/epg-progress")]
async fn api_epg_progress() -> impl Responder {
    if let Ok(progress) = EPG_FETCH_PROGRESS.try_lock() {
        HttpResponse::Ok().json(&*progress)
    } else {
        HttpResponse::InternalServerError().json("Failed to get progress")
    }
}

#[post("/api/fetch-epg")]
async fn api_fetch_epg(args: Data<Args>) -> impl Responder {
    debug!("Manual EPG fetch triggered");
    
    // 使用简化版本获取EPG
        match fetch_all_channels_epg_simple(&args).await {
        Ok(channels) => {
            // 保存到缓存
            if let Ok(mut cache) = ALL_CHANNELS_EPG.try_lock() {
                *cache = Some(channels.clone());
                
                // 保存到文件
                if let Err(e) = save_epg_cache(&channels) {
                    log::error!("Failed to save EPG cache: {}", e);
                } else {
                    log::info!("EPG cache updated and saved");
                }
            }
            
            // 统计信息
            let with_epg = channels.iter().filter(|ch| !ch.epg.is_empty()).count();
            let without_epg = channels.len() - with_epg;
            
            HttpResponse::Ok().json(format!("EPG fetched successfully: {} channels with EPG, {} without EPG", with_epg, without_epg))
        }
        Err(e) => {
            log::error!("Failed to fetch EPG: {}", e);
            HttpResponse::InternalServerError().json(format!("Failed to fetch EPG: {}", e))
        }
    }
}

#[post("/api/regenerate-xmltv")]
async fn api_regenerate_xmltv(args: Data<Args>) -> impl Responder {
    debug!("Manual XMLTV regeneration triggered");
    
    let scheme = "http";
    let host = &args.bind;
    let extra_xml = match &args.extra_xmltv {
        Some(u) => parse_extra_xml(u).await.ok(),
        None => None,
    };
    
    // 使用命令行映射（如果有）
    let cli_mapping = args.channel_mapping.as_ref()
        .map(|s| parse_channel_mapping(s))
        .unwrap_or_default();
    
    match get_channels_with_epg(&args, scheme, host).await {
        Ok(channels) => {
            let channels_clone = channels.clone();
            // 直接使用 to_xmltv 函数，它会自动处理 CHANNEL_MAPPINGS
            match to_xmltv(channels_clone, extra_xml, &cli_mapping) {
                Ok(xmltv) => {
                    // 更新缓存
                    if let Ok(mut cache) = MAPPED_XMLTV_CACHE.try_lock() {
                        *cache = Some(xmltv.clone());
                    }
                    
                    // 保存到文件
                    if let Err(e) = save_xmltv_cache(&xmltv) {
                        log::error!("Failed to save XMLTV cache: {}", e);
                        return HttpResponse::InternalServerError().json("Failed to save cache");
                    }
                    
                    // 统计信息
                    let channel_count = channels.len();
                    let mapped_count = if let Ok(m) = CHANNEL_MAPPINGS.try_lock() {
                        m.len()
                    } else {
                        0
                    };
                    
                    log::info!("XMLTV regenerated successfully: {} channels, {} mappings", channel_count, mapped_count);
                    
                    HttpResponse::Ok().json(format!("XMLTV regenerated successfully: {} channels, {} mappings", channel_count, mapped_count))
                }
                Err(e) => {
                    log::error!("Failed to generate XMLTV: {}", e);
                    HttpResponse::InternalServerError().json(format!("Failed to generate XMLTV: {}", e))
                }
            }
        }
        Err(e) => {
            log::error!("Failed to get channels: {}", e);
            HttpResponse::InternalServerError().json(format!("Failed to get channels: {}", e))
        }
    }
}

#[get("/api/channel-mappings")]
async fn api_get_channel_mappings() -> impl Responder {
    debug!("Getting channel mappings");
    
    if let Ok(mappings) = CHANNEL_MAPPINGS.try_lock() {
        let response: Vec<ChannelMapping> = mappings.iter()
            .map(|(&from_id, &to_id)| ChannelMapping { from_id, to_id })
            .collect();
        HttpResponse::Ok().json(response)
    } else {
        HttpResponse::InternalServerError().json("Failed to get mappings")
    }
}

#[get("/api/channel/{id}/epg")]
async fn api_channel_epg(args: Data<Args>, req: HttpRequest, path: Path<u64>) -> impl Responder {
    debug!("Get channel EPG");
    let channel_id = path.into_inner();
    let scheme = req.connection_info().scheme().to_owned();
    let host = req.connection_info().host().to_owned();
    
    // 检查是否有映射
    let effective_channel_id = if let Ok(mappings) = CHANNEL_MAPPINGS.try_lock() {
        mappings.get(&channel_id).copied().unwrap_or(channel_id)
    } else {
        channel_id
    };
    
    // 首先尝试从缓存获取EPG数据
    if let Ok(cache) = ALL_CHANNELS_EPG.try_lock() {
        if let Some(ref cached_channels) = *cache {
            debug!("Using cached EPG data for channel {}", effective_channel_id);
            if let Some(channel) = cached_channels.iter().find(|c| c.id == effective_channel_id) {
                return HttpResponse::Ok().json(&channel.epg);
            } else {
                // 缓存中没有找到该频道，返回空数组而不是错误
                debug!("Channel {} not found in cache", effective_channel_id);
                return HttpResponse::Ok().json(Vec::<Program>::new());
            }
        }
    }
    
    // 如果没有缓存，实时获取（这种情况应该很少见）
    debug!("No EPG cache available, fetching in real-time");
    match get_channels(&args, true, &scheme, &host).await {
        Ok(channels) => {
            if let Some(channel) = channels.iter().find(|c| c.id == effective_channel_id) {
                HttpResponse::Ok().json(&channel.epg)
            } else {
                HttpResponse::NotFound().json(format!("Channel {} not found", effective_channel_id))
            }
        },
        Err(e) => HttpResponse::InternalServerError().json(format!("Error getting channel EPG: {}", e)),
    }
}

#[get("/api/channels")]
async fn api_channels(args: Data<Args>, req: HttpRequest) -> impl Responder {
    debug!("Get channels");
    let scheme = req.connection_info().scheme().to_owned();
    let host = req.connection_info().host().to_owned();
    
    match get_channels(&args, true, &scheme, &host).await {
        Ok(channels) => HttpResponse::Ok().json(channels),
        Err(e) => HttpResponse::InternalServerError().json(format!("Error getting channels: {}", e)),
    }
}

#[get("/api/channels-with-epg")]
async fn api_channels_with_epg(args: Data<Args>, req: HttpRequest) -> impl Responder {
    debug!("Get channels with EPG");
    let scheme = req.connection_info().scheme().to_owned();
    let host = req.connection_info().host().to_owned();
    
    match get_channels_with_epg(&args, &scheme, &host).await {
        Ok(channels) => HttpResponse::Ok().json(channels),
        Err(e) => HttpResponse::InternalServerError().json(format!("Error getting channels with EPG: {}", e)),
    }
}

#[get("/")]
async fn index() -> impl Responder {
    fs::NamedFile::open_async("/static/index.html").await
}

#[get("/xmltv")]
async fn xmltv_route(args: Data<Args>, req: HttpRequest) -> Result<HttpResponse, Box<dyn std::error::Error>> {
    debug!("Get EPG");
    
    // 首先尝试从缓存获取
    if let Ok(cache) = MAPPED_XMLTV_CACHE.try_lock() {
        if let Some(ref cached_xmltv) = *cache {
            debug!("Returning cached XMLTV");
            return Ok(HttpResponse::Ok().content_type("text/xml").body(cached_xmltv.clone()));
        }
    }
    
    // 如果没有缓存，尝试从文件加载
    if let Ok(Some(file_xmltv)) = load_xmltv_cache() {
        debug!("Loaded XMLTV from file cache");
        if let Ok(mut cache) = MAPPED_XMLTV_CACHE.try_lock() {
            *cache = Some(file_xmltv.clone());
        }
        return Ok(HttpResponse::Ok().content_type("text/xml").body(file_xmltv));
    }
    
    // 如果都没有，实时生成
    let scheme = req.connection_info().scheme().to_owned();
    let host = req.connection_info().host().to_owned();
    let extra_xml = match &args.extra_xmltv {
        Some(u) => parse_extra_xml(u).await.ok(),
        None => None,
    };
    
    let mapping = args.channel_mapping.as_ref()
        .map(|s| parse_channel_mapping(s))
        .unwrap_or_default();
        
    let channels = get_channels_with_epg(&args, &scheme, &host).await?;
    let xml = to_xmltv_with_mappings(channels, extra_xml, &mapping).await?;
    
    // 缓存生成的结果
    if let Ok(mut cache) = MAPPED_XMLTV_CACHE.try_lock() {
        *cache = Some(xml.clone());
        
        // 异步保存到文件
        let xml_for_save = xml.clone();
        tokio::spawn(async move {
            if let Err(e) = save_xmltv_cache(&xml_for_save) {
                log::error!("Failed to save XMLTV cache: {}", e);
            }
        });
    }
    
    Ok(HttpResponse::Ok().content_type("text/xml").body(xml))
}

#[get("/epg.xml")]
async fn epg_xml_cached() -> impl Responder {
    debug!("Get cached EPG XML");
    
    // 首先尝试从内存缓存获取
    if let Ok(cache) = MAPPED_XMLTV_CACHE.try_lock() {
        if let Some(ref cached_xmltv) = *cache {
            debug!("Returning cached XMLTV from memory");
            return HttpResponse::Ok()
                .content_type("text/xml")
                .append_header(("Cache-Control", "public, max-age=21600")) // 6小时
                .body(cached_xmltv.clone());
        }
    }
    
    // 如果内存没有，尝试从文件缓存加载
    match load_xmltv_cache() {
        Ok(Some(file_xmltv)) => {
            debug!("Loaded XMLTV from file cache");
            // 更新内存缓存
            if let Ok(mut cache) = MAPPED_XMLTV_CACHE.try_lock() {
                *cache = Some(file_xmltv.clone());
            }
            HttpResponse::Ok()
                .content_type("text/xml")
                .append_header(("Cache-Control", "public, max-age=21600")) // 6小时
                .body(file_xmltv)
        }
        Ok(None) => {
            // 如果没有缓存，返回空XMLTV
            warn!("No cached EPG data available, returning empty XMLTV");
            let empty_xmltv = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE tv SYSTEM "xmltv.dtd">
<tv generator-info-name="iptv-proxy">
</tv>"#;
            HttpResponse::Ok()
                .content_type("text/xml")
                .body(empty_xmltv.to_string())
        }
        Err(e) => {
            error!("Failed to load XMLTV cache: {}", e);
            HttpResponse::InternalServerError().body(format!("Failed to load EPG cache: {}", e))
        }
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
                        let mapped_id = if let Ok(mappings) = CHANNEL_MAPPINGS.try_lock() {
                            mappings.get(&c.id).copied().unwrap_or_else(|| {
                                // 如果没有前端映射，尝试使用传统的名称映射
                                find_mapped_channel_id(&c.name, &ch, &mapping)
                            })
                        } else {
                            find_mapped_channel_id(&c.name, &ch, &mapping)
                        };
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

#[allow(dead_code)]
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

    // 启动时加载映射
    if let Ok(file_mappings) = load_mappings_from_file() {
        if let Ok(mut mappings) = CHANNEL_MAPPINGS.try_lock() {
            *mappings = file_mappings;
            log::info!("Loaded {} channel mappings from file", mappings.len());
        }
    }
    
    // 加载XMLTV缓存
    let mut has_cache = false;
    if let Ok(Some(xmltv_cache)) = load_xmltv_cache() {
        if let Ok(mut cache) = MAPPED_XMLTV_CACHE.try_lock() {
            *cache = Some(xmltv_cache);
            log::info!("Loaded XMLTV cache from file");
            has_cache = true;
        }
    }
    
    // 加载EPG缓存
    if let Ok(Some(epg_cache)) = load_epg_cache() {
        if let Ok(mut cache) = ALL_CHANNELS_EPG.try_lock() {
            *cache = Some(epg_cache);
            log::info!("Loaded EPG cache from file");
        }
    }
    
    // 如果没有XMLTV缓存，立即生成一个
    if !has_cache {
        log::info!("No XMLTV cache found, generating initial cache...");
        let scheme = "http";
        let host = &args.bind;
        
        // 优先使用缓存的EPG数据
        let channels = if let Ok(cache) = ALL_CHANNELS_EPG.try_lock() {
            if let Some(ref cached_channels) = *cache {
                log::info!("Using cached EPG data for XMLTV generation");
                cached_channels.clone()
            } else {
                match get_channels(&args, true, scheme, host).await {
                    Ok(channels) => channels,
                    Err(e) => {
                        log::error!("Failed to get channels for initial XMLTV: {}", e);
                        return Ok(());
                    }
                }
            }
        } else {
            match get_channels(&args, true, scheme, host).await {
                Ok(channels) => channels,
                Err(e) => {
                    log::error!("Failed to get channels for initial XMLTV: {}", e);
                    return Ok(());
                }
            }
        };
        
        let mapping = args.channel_mapping.as_ref()
            .map(|s| parse_channel_mapping(s))
            .unwrap_or_default();
        
        match to_xmltv(channels, None::<EventReader<Cursor<String>>>, &mapping) {
            Ok(xmltv) => {
                if let Ok(mut cache) = MAPPED_XMLTV_CACHE.try_lock() {
                    *cache = Some(xmltv.clone());
                    
                    // 保存到文件
                    if let Err(e) = save_xmltv_cache(&xmltv) {
                        log::error!("Failed to save XMLTV cache: {}", e);
                    } else {
                        log::info!("Initial XMLTV cache generated and saved");
                    }
                }
            }
            Err(e) => {
                log::error!("Failed to generate initial XMLTV: {}", e);
            }
        }
    }

    let bind_addr = args.bind.clone();
    let args_data = Data::new(args.clone());
    
    // 启动定时任务
    let task_args1 = args_data.clone();
    tokio::spawn(async move {
        generate_mapped_xmltv_periodically(task_args1).await;
    });
    
    // 启动EPG获取定时任务
    let task_args2 = args_data.clone();
    tokio::spawn(async move {
        fetch_all_epg_periodically(task_args2).await;
    });
    
    HttpServer::new(move || {
        let args = args_data.clone();
        App::new()
            .service(index)
            .service(api_channels)
            .service(api_channels_with_epg)
            .service(api_channel_epg)
            .service(api_set_channel_mappings)
            .service(api_get_channel_mappings)
            .service(api_cache_status)
            .service(api_fetch_epg)
            .service(api_regenerate_xmltv)
            .service(xmltv_route)
            .service(epg_xml_cached)
            .service(playlist)
            .service(logo)
            .service(rtsp)
            .service(udp)
            .service(fs::Files::new("/static", "/static").show_files_listing())
            .app_data(args)
    })
    .bind(bind_addr)?
    .run()
    .await
}
