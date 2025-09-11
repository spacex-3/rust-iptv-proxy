# IPTV 代理功能说明

## 🔐 认证功能

为了保护 Web 管理界面不被外网用户随意访问，现已添加 Basic HTTP 认证。

### 默认凭据
- **用户名**: `admin`
- **密码**: `iptv2024`

### 端点访问控制

#### 🔓 开放端点（无需认证）
这些端点是 IPTV 软件必需的，保持开放访问：
- `/playlist` - IPTV 播放列表
- `/epg.xml` - 节目单（缓存版本）
- `/xmltv` - XMLTV 格式节目单  
- `/logo/*.png` - 频道图标
- `/rtsp/*` - RTSP 流转发
- `/udp/*` - UDP 流转发

#### 🔒 受保护端点（需要认证）
这些端点需要输入用户名密码：
- `/` - Web 管理主页
- `/static/*` - 所有静态 Web 资源
- `/api/*` - 所有管理 API 端点

## 🌍 IP 地理位置功能

项目已集成 IP 地理位置查询功能，自动为播放记录添加地理位置信息：

### API 服务
- **使用服务**: ip-api.com
- **查询格式**: `http://ip-api.com/json/{IP}?lang=zh-CN`
- **显示格式**: `regionName-city-isp`
- **示例**: `广东-广州市-Chinanet`

### 特殊处理
- **本地网络**: 192.168.x.x, 10.x.x.x, 172.x.x.x 显示为 "本地网络"
- **查询失败**: 显示为 "未知地区"
- **服务器端查询**: 在记录播放行为时自动查询，避免前端重复请求

### API 响应示例
```json
{
  "status": "success",
  "country": "中国",
  "regionName": "广东",
  "city": "广州市",
  "isp": "Chinanet",
  "query": "121.8.215.106"
}
```

## 🛠️ 修改配置

### 修改认证凭据
如需修改用户名或密码，请编辑 `src/main.rs` 文件中的 `AuthMiddlewareService` 实现：

```rust
// 在第 1636 行附近找到这段代码
if username == "admin" && password == "iptv2024" {
    authenticated = true;
}
```

将 `"admin"` 和 `"iptv2024"` 替换为您希望的用户名和密码，然后重新构建 Docker 镜像。

### 修改 IP 地理位置 API
如需更换地理位置 API，请编辑 `src/main.rs` 文件中的 `get_ip_location` 函数。

## 🧪 测试功能

### 测试认证功能
```bash
./test-auth.sh
```

### 测试 IP 地理位置功能
```bash
./test-ip-location.sh
```

## 🌐 外网访问配置

通过 Lucky 反向代理后：

### IPTV 软件配置
- **源地址**: `http://你的域名/playlist`
- **节目单**: `http://你的域名/epg.xml`

### Web 管理界面
- **访问**: `http://你的域名/`
- **认证**: 会弹出认证对话框，输入用户名密码即可

## 🔒 安全建议

1. **定期更换密码** - 建议每月更换一次管理密码
2. **使用强密码** - 密码应包含字母、数字和特殊字符
3. **监控访问日志** - 定期检查 Docker 容器日志
4. **考虑 HTTPS** - 在生产环境中建议配置 SSL 证书
5. **限制访问源** - 可在反向代理层面限制访问 IP

## 📊 数据存储

### 持久化文件
这些文件保存在宿主机的 `data` 目录：
- `playback_stats.json` - 播放统计记录
- `channel_mappings.json` - 频道映射配置
- `xmltv_cache.xml` - XMLTV 节目单缓存

### 备份建议
- 定期备份 `data` 目录
- 可使用 `backup.sh` 和 `restore.sh` 脚本