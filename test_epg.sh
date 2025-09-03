#!/bin/bash

# 设置环境变量
export RUST_LOG=debug
export RUST_BACKTRACE=1

# 编译
echo "正在编译..."
cargo build --release

if [ $? -ne 0 ]; then
    echo "编译失败"
    exit 1
fi

echo "编译成功，启动服务..."

# 创建日志目录
mkdir -p logs

# 启动服务并保存日志
nohup ./target/release/iptv \
    --user your_username \
    --passwd your_password \
    --mac 00:00:00:00:00:00 \
    --bind 0.0.0.0:7878 \
    > logs/iptv.log 2>&1 &

echo "服务已启动，PID: $!"
echo "日志文件: logs/iptv.log"
echo "访问地址: http://localhost:7878"

# 等待5秒
sleep 5

# 测试EPG获取
echo "测试EPG获取..."
curl -X POST http://localhost:7878/api/fetch-epg

echo -e "\n\n检查日志最后100行："
tail -100 logs/iptv.log