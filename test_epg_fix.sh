#!/bin/bash

echo "=== IPTV EPG 测试脚本 ==="
echo

# 构建项目
echo "正在构建..."
cargo build --release

if [ $? -ne 0 ]; then
    echo "构建失败"
    exit 1
fi

echo "构建成功！"
echo

# 创建日志目录
mkdir -p logs

# 启动服务
echo "启动服务..."
nohup ./target/release/iptv \
    --user your_username \
    --passwd your_password \
    --mac 00:00:00:00:00:00 \
    --bind 0.0.0.0:7878 \
    > logs/iptv.log 2>&1 &

IPTV_PID=$!
echo "服务已启动，PID: $IPTV_PID"
echo "日志文件: logs/iptv.log"
echo

# 等待服务启动
echo "等待服务启动..."
sleep 5

# 测试服务是否运行
if curl -s http://localhost:7878/api/channels > /dev/null; then
    echo "✅ 服务运行正常"
else
    echo "❌ 服务启动失败"
    kill $IPTV_PID 2>/dev/null
    exit 1
fi

echo
echo "开始测试EPG获取..."
echo

# 获取频道总数
TOTAL_CHANNELS=$(curl -s http://localhost:7878/api/channels | jq '. | length')
echo "总频道数: $TOTAL_CHANNELS"

# 测试获取所有EPG
echo -e "\n1. 测试获取所有EPG..."
echo "发送请求到 /api/fetch-epg"
curl -X POST http://localhost:7878/api/fetch-epg

# 等待处理
echo -e "\n\n等待10秒..."
sleep 10

# 检查日志
echo -e "\n2. 检查EPG获取日志..."
echo "=========================="
tail -50 logs/iptv.log | grep -E "(EPG fetch completed|Got.*programs|Failed to parse)"
echo "=========================="

# 测试单个频道EPG
echo -e "\n3. 测试单个频道EPG..."
echo "测试CCTV-1 (ID: 671842865)"
curl -s http://localhost:7878/api/channel/671842865/epg | jq '.[0] // "无EPG数据"'

# 检查XMLTV
echo -e "\n4. 检查XMLTV文件..."
if curl -s http://localhost:7878/xmltv | head -c 200 | grep -q "<tv"; then
    echo "✅ XMLTV文件可访问"
    echo "文件大小: $(curl -s http://localhost:7878/xmltv | wc -c) 字节"
else
    echo "❌ XMLTV文件不可访问"
fi

# 清理
echo -e "\n\n清理..."
kill $IPTV_PID 2>/dev/null
echo "服务已停止"

echo -e "\n测试完成！查看详细日志: tail -f logs/iptv.log"