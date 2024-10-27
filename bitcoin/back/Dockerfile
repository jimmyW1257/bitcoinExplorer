#这行指定了基础镜像，使用的是最新版本的 Rust 官方 Docker 镜像
FROM rust:latest

#设置了工作目录。当后续的命令运行时，都会在这个目录下执行。如果目录不存在，Docker 会自动创建它。
WORKDIR /root/bitcoin-back

#这个命令将当前上下文中的所有文件（即 Dockerfile 所在目录中的所有文件）复制到 Docker 镜像的工作目录（即 /root/bitcoin-back）。
COPY . .

#这行命令使用 Rust 的包管理工具 cargo 来构建应用。生成一个优化后的可执行文件。它会在 target/release 目录下生成可执行文件，通常用于生产环境。
RUN cargo build --release

#这行命令指定了容器启动时要执行的命令。
CMD ["./target/release/bitcoin-explorer"]
