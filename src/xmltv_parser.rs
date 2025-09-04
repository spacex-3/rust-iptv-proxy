use std::collections::HashMap;
use std::io::Cursor;
use xml::{reader::XmlEvent as XmlReadEvent, EventReader};
use anyhow::{anyhow, Result};
use log::debug;
use crate::Program;

// 从XMLTV缓存中解析EPG数据
pub fn parse_epg_from_xmltv(xmltv_content: &str) -> Result<HashMap<u64, Vec<Program>>> {
    let reader = Cursor::new(xmltv_content);
    let parser = EventReader::new(reader);
    
    let mut epg_data = HashMap::new();
    let mut current_channel_id = None;
    let mut current_program = None;
    let mut in_title = false;
    let mut in_desc = false;
    let mut title = String::new();
    let mut desc = String::new();
    
    for event in parser {
        match event {
            Ok(XmlReadEvent::StartElement { name, attributes, .. }) => {
                match name.local_name.as_str() {
                    "programme" => {
                        // 解析频道ID
                        let channel_attr = attributes.iter()
                            .find(|attr| attr.name.local_name == "channel")
                            .map(|attr| &attr.value);
                        
                        if let Some(channel_str) = channel_attr {
                            // 去掉引号
                            let channel_id = channel_str.trim_matches('"')
                                .parse::<u64>()
                                .unwrap_or(0);
                            
                            let start = attributes.iter()
                                .find(|attr| attr.name.local_name == "start")
                                .and_then(|attr| parse_xmltv_time(&attr.value).ok())
                                .unwrap_or(0);
                            
                            let stop = attributes.iter()
                                .find(|attr| attr.name.local_name == "stop")
                                .and_then(|attr| parse_xmltv_time(&attr.value).ok())
                                .unwrap_or(0);
                            
                            if channel_id > 0 && start > 0 && stop > 0 {
                                current_channel_id = Some(channel_id);
                                current_program = Some((start, stop));
                                title.clear();
                                desc.clear();
                            }
                        }
                    }
                    "title" => {
                        in_title = true;
                        title.clear();
                    }
                    "desc" => {
                        in_desc = true;
                        desc.clear();
                    }
                    _ => {}
                }
            }
            Ok(XmlReadEvent::Characters(content)) => {
                if in_title {
                    title.push_str(&content);
                } else if in_desc {
                    desc.push_str(&content);
                }
            }
            Ok(XmlReadEvent::EndElement { name }) => {
                match name.local_name.as_str() {
                    "title" => {
                        in_title = false;
                    }
                    "desc" => {
                        in_desc = false;
                        // desc结束，保存节目
                        if let Some((start, stop)) = current_program {
                            if let Some(channel_id) = current_channel_id {
                                let program = Program {
                                    start,
                                    stop,
                                    title: title.clone(),
                                    desc: if desc.is_empty() { title.clone() } else { desc.clone() },
                                };
                                epg_data.entry(channel_id).or_insert_with(Vec::new).push(program);
                            }
                        }
                        current_program = None;
                        current_channel_id = None;
                    }
                    "programme" => {
                        // 如果没有desc，只在title结束后保存
                        if current_program.is_some() && !title.is_empty() {
                            if let Some((start, stop)) = current_program {
                                if let Some(channel_id) = current_channel_id {
                                    let program = Program {
                                        start,
                                        stop,
                                        title: title.clone(),
                                        desc: title.clone(),
                                    };
                                    epg_data.entry(channel_id).or_insert_with(Vec::new).push(program);
                                }
                            }
                        }
                        current_program = None;
                        current_channel_id = None;
                    }
                    _ => {}
                }
            }
            Err(e) => {
                debug!("XML parsing error: {}", e);
            }
            _ => {}
        }
    }
    
    debug!("Parsed EPG data for {} channels", epg_data.len());
    Ok(epg_data)
}

// 解析XMLTV时间格式
fn parse_xmltv_time(time_str: &str) -> Result<i64> {
    // 去掉时区信息
    let time_str = time_str.trim_end_matches(" +0800").trim();
    let format = "%Y%m%d%H%M%S";
    chrono::DateTime::parse_from_str(time_str, format)
        .map(|dt| dt.timestamp_millis())
        .map_err(|e| anyhow!("Failed to parse time '{}': {}", time_str, e))
}