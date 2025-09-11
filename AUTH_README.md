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