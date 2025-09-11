# Rust IPTV Proxy

一个用 Rust 编写的广东电信 IPTV 代理服务器，支持频道映射和多种代理模式。

## 快速开始

### 使用 Docker Compose (推荐)

1. **下载配置文件**
   ```bash
   wget https://raw.githubusercontent.com/your-repo/rust-iptv-proxy/master/docker-compose.template.yml -O docker-compose.yml
   wget https://raw.githubusercontent.com/your-repo/rust-iptv-proxy/master/.env.template -O .env
   ```

2. **配置环境变量**
   编辑 `.env` 文件，填入你的 IPTV 账户信息：
   ```env
   IPTV_USER=your_username
   IPTV_PASSWD=your_password
   IPTV_MAC=your_mac_address
   ```

3. **启动服务**
   ```bash
   docker-compose up -d
   ```

4. **访问服务**
   - 播放列表: http://localhost:7878/playlist
   - EPG 节目单: http://localhost:7878/xmltv
   - 频道 Logo: http://localhost:7878/logo/{channel_id}.png

### 直接使用 Docker

```bash
docker run -d \
  --name iptv-proxy \
  -p 7878:7878 \
  your-dockerhub-username/rust-iptv-proxy:latest \
  --user your_username \
  --passwd your_password \
  --mac your_mac_address \
  --bind 0.0.0.0:7878 \
  --rtsp-proxy
```

## 配置选项

### 基本参数
- `--user`: IPTV 登录用户名
- `--passwd`: IPTV 登录密码
- `--mac`: MAC 地址
- `--bind`: 绑定地址和端口 (默认: 0.0.0.0:7878)

### 代理模式
- `--rtsp-proxy`: 启用 RTSP 代理模式
- `--udp-proxy`: 启用 UDP 代理模式

### 频道映射
使用 `--channel-mapping` 参数让高清频道复用标清频道的 logo 和 EPG：

```bash
--channel-mapping "CCTV-1综合高清=CCTV-1综合,CCTV-2财经高清=CCTV-2财经"
```

### 扩展功能
- `--extra-playlist`: 额外的 M3U 播放列表 URL
- `--extra-xmltv`: 额外的 XMLTV EPG URL
- `--interface`: 指定网络接口
- `--address`: 指定 IP 地址

## 示例配置

### 完整的 docker-compose.yml
```yaml
version: '3.8'

services:
  iptv-proxy:
    image: your-dockerhub-username/rust-iptv-proxy:latest
    ports:
      - "7878:7878"
    command: [
      "--user", "your_username",
      "--passwd", "your_password",
      "--mac", "your_mac_address",
      "--bind", "0.0.0.0:7878",
      "--rtsp-proxy",
      "--channel-mapping", "CCTV-1综合高清=CCTV-1综合,CCTV-2财经高清=CCTV-2财经"
    ]
    environment:
      - RUST_LOG=info
    restart: unless-stopped
```

## 网络模式

如果需要 UDP 多播支持，启用 host 网络模式：
```yaml
network_mode: host
```

## 日志级别

通过环境变量 `RUST_LOG` 控制日志级别：
- `debug`: 详细调试信息
- `info`: 一般信息 (默认)
- `warn`: 警告信息
- `error`: 仅错误信息

## 支持的 URL 端点

- `/playlist` - M3U8 播放列表
- `/xmltv` - XMLTV 格式的 EPG 数据
- `/logo/{id}.png` - 频道 Logo 图片
- `/rtsp/{path}` - RTSP 流代理
- `/udp/{address}` - UDP 流代理



# IPTV 代理认证配置说明

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

## 🛠️ 修改认证凭据

如需修改用户名或密码，请编辑 `src/main.rs` 文件中的 `auth_middleware` 函数：

```rust
// 在第 1597 行附近找到这段代码
if username == "admin" && password == "iptv2024" {
    return next(req).await;
}
```

将 `"admin"` 和 `"iptv2024"` 替换为您希望的用户名和密码，然后重新构建 Docker 镜像。

## 🧪 测试认证功能

运行测试脚本验证认证配置：

```bash
./test-auth.sh
```

## 🌐 外网访问配置

通过 Lucky 反向代理后：
1. **IPTV 软件配置**：
   - 源地址：`http://你的域名/playlist`
   - 节目单：`http://你的域名/epg.xml`

2. **Web 管理界面**：
   - 访问：`http://你的域名/`
   - 会弹出认证对话框，输入用户名密码即可

## 🔒 安全建议

1. 定期更换密码
2. 使用强密码
3. 考虑添加 HTTPS 支持
4. 监控访问日志

## 许可证

本项目使用 MIT 许可证。