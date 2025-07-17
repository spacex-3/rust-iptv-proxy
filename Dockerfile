FROM rust:latest AS builder
RUN curl https://sh.rustup.rs -sSf | sh -s -- -y
ENV PATH="/root/.cargo/bin:${PATH}"
RUN rustup update stable

# 设置工作目录
WORKDIR /app

# 复制依赖文件
COPY Cargo.toml Cargo.lock ./
RUN cargo --version
# 创建一个虚拟的 main.rs 来预编译依赖项
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo build --release
RUN rm src/main.rs

# 复制源代码
COPY src ./src

# 构建应用程序
RUN cargo build --release

# 使用更小的运行时镜像
FROM debian:bookworm-slim

# 安装运行时依赖
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

# 复制编译好的二进制文件
COPY --from=builder /app/target/release/iptv /usr/local/bin/iptv

# 复制静态文件到容器
COPY static /static

# 暴露默认端口
EXPOSE 7878

# 设置入口点
ENTRYPOINT ["/usr/local/bin/iptv"]