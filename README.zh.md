<div align="center">

# OpenKache ⚡

**OpenKache is a high-performance cache server designed from the ground up for modern SSDs.**

开源 · RESP/TCP · OpenKache/QUIC · Linux `io_uring`

[![Build](https://img.shields.io/badge/build-preview-orange.svg)](https://github.com/openkache/openkache/actions)
[![Rust](https://img.shields.io/badge/rust-2024-orange.svg)](https://www.rust-lang.org/)

[English](./README.md) · [한국어](./README.ko.md) · **中文**

</div>

## 目录

- [基准测试](#基准测试)
- [架构](#架构)
- [快速开始](#快速开始)
- [连接客户端](#连接客户端)
- [容器镜像](#容器镜像)
- [路线图](#路线图)
- [构建与验证](#构建与验证)
- [客户端包](#客户端包)
- [仓库结构](#仓库结构)
- [项目状态](#项目状态)
- [参与贡献](#参与贡献)
- [许可证](#许可证)
- [第三方归属声明](#第三方归属声明)

## 基准测试

基准测试在 [serveroptima1](./benchmark/BENCHMARK.md#test-environment) 上通过回环网络运行。

三个系统都使用 kvbench 通过各自的原生协议进行基准测试。由于每个数据库的协议不同,
我们构建了 kvbench 来用同一标准测量它们。完整方法和 kvbench 说明见
[benchmark/BENCHMARK.md](./benchmark/BENCHMARK.md)。

**GET 吞吐量**

| 系统 | GET 吞吐量 | 负载工具 |
|---|---:|---|
| OpenKache | **97,887 ops/s (1×)** | kvbench (RESP) |
| PostgreSQL 17.10 | 17,421 ops/s (0.18×) | kvbench (PostgreSQL wire) |
| MySQL 8.4.11 | 16,295 ops/s (0.17×) | kvbench (MySQL wire) |

OpenKache 达到硬件上限的 76%(128,820 IOPS,由
[fio](https://github.com/axboe/fio) 测得)。

**GET 延迟(单次串行请求)**

| 系统 | 平均 | p50 | p99 | p99.9 |
|---|---:|---:|---:|---:|
| OpenKache | **238.7 µs (1×)** | 229 µs (1×) | 386 µs (1×) | 1376 µs (1×) |
| MySQL 8.4.11 | 385.7 µs (1.6×) | 410 µs (1.8×) | 1169 µs (3.0×) | 2207 µs (1.6×) |
| PostgreSQL 17.10 | 558.0 µs (2.3×) | 510 µs (2.2×) | 1263 µs (3.3×) | 3342 µs (2.4×) |

## 架构

<div align="center">

<img src="docs/assets/openkache-architecture.png" alt="OpenKache 架构"/>

</div>

**OpenKache 为什么快?** 因为它从不在核心之间跳转。

大多数服务器让线程池在核心之间游走。这个代价并不便宜 —— 锁竞争、互斥量、上下文切换,以及
每当缓存行在核心之间弹跳时都要付出的同步与拷贝成本。负载越重,这些开销就越是吞噬吞吐量。

OpenKache 采用 **thread-per-core(shared-nothing,无共享)** 设计。每个工作线程被绑定到
单个核心,只拥有自己的数据,不共享任何状态 —— 因此没有锁。这正是 TigerBeetle、ScyllaDB、
Redis 为榨干硬件性能而共同收敛到的设计。网络路径与存储路径各自独占一个核心,两者之间只通过
一条 **无锁 SPSC 队列** 通信。RESP 解析绝不会阻塞磁盘 I/O。

Redis 在单一核心上执行命令。OpenKache 保持同样的无共享原则,但将工作线程按核心分片,使吞吐量
随硬件扩展,而不是停在单核天花板上 —— 由于没有共享锁,增加核心也不会带来竞争。

值存放在 SSD 上,键存放在 RAM 的紧凑索引中(压缩键 → 段偏移)。正如地铁比汽车运送更多乘客,
OpenKache 把许多键的写入合并成一次顺序的 **段组(segment-group)** 刷写,而不是每个键单独写
一次 SSD,从而最大限度地利用磁盘的顺序带宽 —— 在 Linux 上,通过 `io_uring` 提交这些 I/O,
连系统调用开销也一并抹去。

这一切都用 **Rust** 编写:没有 GC 停顿,数据竞争在编译期被排除,同时握有 C 级别的控制力。
快路径上没有任何垃圾回收停顿的容身之处。

完整设计见 [docs/architecture.md](./docs/architecture.md)。(中文翻译准备中)

## 快速开始

OpenKache 针对 **Linux** 进行优化和基准测试。

- **Windows：** 建议使用 WSL2。
- **macOS：** 仅用于功能开发，不用于性能比较。

Linux 要求:

- 支持 `io_uring` 的 Linux
- 进程可用的两个不同 CPU

### Docker

下载镜像：

```bash
docker pull ghcr.io/openkache/openkache:edge
```

运行服务器：

```bash
docker run --rm \
  --network host \
  --security-opt seccomp=unconfined \
  ghcr.io/openkache/openkache:edge
```

### 下载并运行

OpenKache 尚未发布正式版本。从 [server-v0.1.0](https://github.com/openkache/openkache/releases/tag/server-v0.1.0)
下载并解压源码，然后运行下方的 Cargo 命令。

### Cargo

```bash
cargo run --locked --package openkache-server --bin openkache-server
```

## 连接客户端

服务器运行后,用你选择的语言连接。所有客户端指南都默认使用 `127.0.0.1:4433` 作为本地端点,
并列出各自语言的完整公开 API。

| 包 | 安装 | 文档 | 源码 |
|---|---|---|---|
| TypeScript / JavaScript | `npm install openkache` | [npm](https://www.npmjs.com/package/openkache) · [客户端 README](clients/typescript/README.md) | [GitHub](https://github.com/openkache/openkache/tree/main/clients/typescript) |
| Python | `python -m pip install openkache` | [PyPI](https://pypi.org/project/openkache/) · [客户端 README](clients/python/README.md) | [GitHub](https://github.com/openkache/openkache/tree/main/clients/python) |
| Rust | `cargo add openkache` | [crates.io](https://crates.io/crates/openkache) · [docs.rs](https://docs.rs/openkache/latest/openkache/) · [客户端 README](clients/rust/README.md) | [GitHub](https://github.com/openkache/openkache/tree/main/clients/rust) |

Rust SDK 速览:

```rust
use openkache::{Client, Value};

# async fn example() -> openkache::Result<()> {
let client = Client::connect("127.0.0.1:4433").await?;
client.set("greeting", Value::text("hello")).await?;
assert_eq!(
    client.get("greeting").await?.unwrap(),
    Value::text("hello"),
);
client.close().await?;
# Ok(())
# }
```

从源码构建的 [`openkache-cli`](clients/cli/README.md) 是 Bash 友好的选项,默认使用同样固定的
Gate 0 配置:

```bash
openkache-cli set hello "from cli"
openkache-cli get hello
```

当需要证书根、双向 TLS、客户端侧值保护,或仅为兼容性而设的 TTL/条件写入时,请使用
`openkache-cli --profile configured`。

> **本地开发信任配置。** 默认的 Gate 0 配置用于本地开发:它在 QUIC 之上使用 TLS 1.3 且绝不
> 回退到明文,但不验证服务器证书。请勿将这种信任配置用于生产流量。

## 路线图

| 里程碑 | 状态 | 重点 |
|---|---|---|
| SSD 存储 & 双协议服务器 | 🚧 进行中 | 段组写入,RESP/TCP + QUIC Gate 0 |
| 生产加固 | 🔜 下一步 | 重启恢复、可复现基准测试、模糊测试、CI/CD |
| 安全 & 正确性 | 📅 计划中 | 认证/mTLS、值保护配置、完整协议面 |
| 扩展 & 覆盖 | 📅 计划中 | 集群、跨平台服务器、正式发布(GA) |

详情见 [ROADMAP.md](./ROADMAP.md)。(中文翻译准备中)

## 构建与验证

```bash
cargo check --locked
cargo test --locked --package openkache-server
cargo server-build
```

根 Cargo 工作区在同一个锁文件下管理协议、服务器、共享客户端核心、Rust SDK、CLI 以及原生
TypeScript 适配器。

服务器分配器(allocator)相关实验作为可选功能提供:

```bash
cargo server-build --features alloc-jemalloc
cargo server-build --features alloc-mimalloc
```

不要同时启用这两个分配器功能。

## 客户端包

受维护的客户端包共享同一套协议与值格式(value-format)源码。Rust、TypeScript、Python、
.NET、Go、C、C++、Swift 等绑定的当前状态见 [clients/README.md](./clients/README.md)。

当前服务器的兼容性前端仅支持上文列出的 Gate 0 操作子集。目标契约(target contract)中描述的
更广泛 API,可能会先出现在生成的客户端中,而服务器尚未实现。

## 仓库结构

| 路径 | 内容 |
| --- | --- |
| `server/` | 当前的 SSD 缓存服务器与容器定义 |
| `protocol/` | 共享的传输模型、生成的契约与编解码器 |
| `clients/` | 客户端 SDK 与原生适配器 |
| `docs/` | 当前的使用指南以及明确标识的目标文档 |

当前的服务器实现见 [server/README.md](./server/README.md)。协议细节见
[protocol/README.md](./protocol/README.md)。

## 项目状态

| 组件 | 状态 |
| --- | --- |
| RESP/TCP 服务器 | 预览版 |
| OpenKache/QUIC Gate 0 服务器 | 预览版 |
| SSD 存储与删除 | 预览版 |
| 重启恢复 | 未实现 |
| 生产环境认证 | 未实现 |
| 客户端 SDK | 预览版;详见各包状态 |
| 容器镜像 | 支持 Linux amd64/arm64 |
| 集群 | 尚未开始 |

## 参与贡献

- [贡献指南](./CONTRIBUTING.md)
- [社区准则](./COMMUNITY_GUIDELINES.md)
- [行为准则](./CODE_OF_CONDUCT.md)

## 许可证

除非另有说明,OpenKache 依据
[GNU Affero 通用公共许可证 v3.0 或更高版本](./LICENSE)授权。[`clients/`](./clients/) 下的
客户端 SDK 与 [`protocol/`](./protocol/) 下的共享协议依据 Apache License 2.0 授权;详见各
目录下的 `LICENSE` 文件。

## 第三方归属声明

发布归档、容器镜像和客户端软件包包含根据锁定的 Cargo 依赖关系图生成的
`THIRD-PARTY-NOTICES.txt`。该文件保留上游许可证、声明和归属文本;OpenKache 自身的
许可证位于相邻的 `LICENSE` 文件中。

没有独立上游许可证文件的条目会被标记为需要维护者审查。若声明中仍包含
`LEGAL REVIEW REQUIRED`,请勿重新分发该构件。发布检查和各构件的具体说明见
[`RELEASING.md`](./RELEASING.md)。
