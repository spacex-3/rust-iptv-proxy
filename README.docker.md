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

## 许可证

本项目使用 MIT 许可证。