# 数据目录说明

这个目录包含了IPTV代理服务的持久化数据文件，便于备份和迁移。

## 文件说明

### `playback_stats.json`
- **用途**: 播放统计记录
- **内容**: 包含所有播放记录，包括时间戳、客户端IP、地理位置、频道信息等
- **格式**: JSON数组，每个元素是一条播放记录
- **示例**:
```json
[
  {
    "timestamp": 1757245955093,
    "client_ip": "127.0.0.1",
    "channel_id": "3221229774",
    "channel_name": "CCTV -1综合高清(RTSP匹配)",
    "user_agent": "curl/8.7.1",
    "rtsp_url": "rtsp://...",
    "ip_location": "本地网络"
  }
]
```

### `channel_mappings.json`
- **用途**: 频道映射配置
- **内容**: 存储通过Web界面设置的频道ID映射关系
- **格式**: JSON对象，键值对表示源频道ID到目标频道ID的映射
- **示例**:
```json
{
  "123456": 789012,
  "234567": 890123
}
```

### `xmltv_cache.xml`
- **用途**: EPG节目单缓存
- **内容**: 缓存的XMLTV格式节目单数据
- **格式**: 标准XMLTV XML格式
- **大小**: 通常几MB，包含所有频道的节目信息

## 备份建议

```bash
# 备份整个数据目录
tar -czf iptv-data-backup-$(date +%Y%m%d).tar.gz data/

# 只备份重要文件
cp data/playback_stats.json backup/
cp data/channel_mappings.json backup/
```

## 迁移到新环境

1. 停止当前服务：
```bash
docker-compose down
```

2. 复制数据目录到新环境：
```bash
scp -r data/ user@newserver:/path/to/iptv-proxy/
```

3. 在新环境启动服务：
```bash
docker-compose up -d
```

## 注意事项

- 这些文件会被Docker容器自动映射到容器内的对应位置
- 如果文件不存在，服务启动时会自动创建空文件
- 建议定期备份`playback_stats.json`，因为它包含历史播放数据
- `xmltv_cache.xml`会定期自动更新，无需手动维护
- `channel_mappings.json`只有在Web界面设置映射后才会创建