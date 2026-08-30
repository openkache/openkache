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
- [路线图](#路线图)
- [参与贡献](#参与贡献)
- [许可证](#许可证)
- [第三方归属声明](#第三方归属声明)

---

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

---

## 架构

<div align="center">

<img src="docs/assets/openkache-architecture.png" alt="OpenKache 架构"/>

</div>

每个请求都经过一条短而可预测的路径：

1. 客户端通过 RESP/TCP 或 OpenKache/QUIC 发送 `GET`、`SET` 或 `DELETE`。
2. 固定在一个核心上的网络工作线程解析请求，不接触存储。
3. 请求通过一条无锁的 **SPSC（single-producer, single-consumer）** 队列传给同样固定在
   一个核心上的存储工作线程。
4. 存储工作线程使用紧凑的 RAM 表选择一个候选 4 KiB 桶，然后验证桶内的 32 字节存储键。
   可变桶已在 RAM 中；flush 后的桶从 SSD 读取。

### 为什么快

- **热点状态留在本地核心。** 每个工作线程拥有自己的数据并固定在一个核心上，从而减少共享
  锁和缓存行移动。这与使用 shared-nothing 原则来充分利用现代 NVMe 设备的
  [SOSP '19 · KVell](https://github.com/BLepers/KVell/blob/master/sosp19-final40.pdf)方向一致。
- **RAM 存储位置线索，而不是键。** 查询表只保留 8 位 fingerprint 和紧凑的段/桶候选位置。
  候选桶会验证值旁边完整的 32 字节存储键，因此 RAM 用于微小的路由元数据，而不是保存每个
  键的副本（[SIGMOD '26 · Breadcrumb Filters](https://doi.org/10.1145/3786629)）。
- **把小写入变成顺序批次。** 段组不会为每个请求分别发出一次 SSD 写入，而是把多个键的
  写入合并成一次更大的 flush。混合闪存缓存也用这一思路把微小对象写入转换成大块顺序 I/O
  （[NSDI '19 · Flashield](https://www.usenix.org/conference/nsdi19/presentation/eisenman)，
  [ASPLOS '26 · Nemo](https://github.com/XMU-DISCLab/Cachelib-Nemo)）。
- **Linux 使用 `io_uring`。** 网络工作线程继续处理请求时，OpenKache 会批量处理异步 I/O
  的提交与完成。

服务器使用 Rust 编写，以明确管理请求路径上的内存所有权，并避免使用垃圾收集器。

完整设计见 [docs/architecture.md](./docs/architecture.md)。(中文翻译准备中)

---

## 快速开始

安装程序会下载最新的标签版本，为 Linux x86-64、Linux ARM64 或 Apple Silicon
macOS 选择正确的压缩包，验证其 SHA-256 校验和，然后将 `openkache-server` 和
`openkache-cli` 安装到 `~/.local/bin`。Windows 用户可以在 WSL2 中运行 Linux 版本。

### 安装 OpenKache

```bash
curl -fsSL https://github.com/openkache/openkache/raw/main/install.sh | sh
```

运行前可以先[查看安装程序](./install.sh)。

如果 shell 提示 `command not found`，请为当前终端将安装目录添加到 `PATH`：

```bash
export PATH="$HOME/.local/bin:$PATH"
```

Linux 需要 `io_uring` 和两个可用 CPU。Apple Silicon macOS 二进制文件使用原生功能
开发路径；性能数据仅适用于 Linux。

### 终端 1：启动服务器

```bash
openkache-server
```

使用 OpenKache 时，请保持此终端开启。

### 终端 2：验证服务器

打开第二个终端并运行：

```bash
openkache-cli ping
# PONG

openkache-cli set hello "from CLI"
# CREATED

openkache-cli get hello
# from CLI
```

返回终端 1，按 <kbd>Ctrl</kbd>+<kbd>C</kbd> 停止服务器。

---

## 连接客户端

服务器运行后,用你选择的语言连接。所有客户端指南都默认使用 `127.0.0.1:4433` 作为本地端点,
并列出各自语言的完整公开 API。

OpenKache 分别提供 TypeScript/JavaScript、Python 和 Rust 客户端包。三个包共享同一套协议和
value-format 源码:

| 包 | 安装 | 文档 | 源码 |
|---|---|---|---|
| TypeScript / JavaScript | `npm install openkache` | [npm](https://www.npmjs.com/package/openkache) · [客户端 README](clients/typescript/README.md) | [GitHub](https://github.com/openkache/openkache/tree/main/clients/typescript) |
| Python | `python -m pip install openkache` | [PyPI](https://pypi.org/project/openkache/) · [客户端 README](clients/python/README.md) | [GitHub](https://github.com/openkache/openkache/tree/main/clients/python) |
| Rust | `cargo add openkache` | [crates.io](https://crates.io/crates/openkache) · [docs.rs](https://docs.rs/openkache/latest/openkache/) · [客户端 README](clients/rust/README.md) | [GitHub](https://github.com/openkache/openkache/tree/main/clients/rust) |

.NET、Go、C、C++、Swift 等其他绑定的状态见 [clients/README.md](./clients/README.md)。

Rust SDK 速览:

```rust
use openkache::{Client, Value};

async fn example() -> openkache::Result<()> {
    let client = Client::connect("127.0.0.1:4433").await?;
    client.set("greeting", Value::text("hello")).await?;
    assert_eq!(
        client.get("greeting").await?.unwrap(),
        Value::text("hello"),
    );
    client.close().await?;
    Ok(())
}
```

每个标签服务器版本都包含 [`openkache-cli`](clients/cli/README.md) 二进制文件，适合在
Bash 中使用。它默认使用相同的固定 Gate 0 配置；完整示例请参阅[快速开始](#快速开始)。

当需要证书根、双向 TLS、客户端侧值保护,或仅为兼容性而设的 TTL/条件写入时,请使用
`openkache-cli --profile configured`。

---

## 路线图

| 里程碑 | 状态 | 重点 |
|---|---|---|
| SSD 存储 & 双协议服务器 | 🚧 进行中 | 段组写入,RESP/TCP + QUIC Gate 0 |
| 生产加固 | 🔜 下一步 | 重启恢复、可复现基准测试、模糊测试、CI/CD |
| 安全 & 正确性 | 📅 计划中 | 认证/mTLS、值保护配置、完整协议面 |
| 扩展 & 覆盖 | 📅 计划中 | 集群、跨平台服务器、正式发布(GA) |

详情见 [ROADMAP.md](./ROADMAP.md)。(中文翻译准备中)

---

## 参与贡献

- [贡献指南](./CONTRIBUTING.md)
- [社区准则](./COMMUNITY_GUIDELINES.md)
- [行为准则](./CODE_OF_CONDUCT.md)

---

## 许可证

除非另有说明,OpenKache 依据
[GNU Affero 通用公共许可证 v3.0 或更高版本](./LICENSE)授权。[`clients/`](./clients/) 下的
客户端 SDK 与 [`protocol/`](./protocol/) 下的共享协议依据 Apache License 2.0 授权;详见各
目录下的 `LICENSE` 文件。

---

## 第三方归属声明

发布归档、容器镜像和客户端软件包包含根据锁定的 Cargo 依赖关系图生成的
`THIRD-PARTY-NOTICES.txt`。该文件保留上游许可证、声明和归属文本;OpenKache 自身的
许可证位于相邻的 `LICENSE` 文件中。

没有独立上游许可证文件的条目会被标记为需要维护者审查。若声明中仍包含
`LEGAL REVIEW REQUIRED`,请勿重新分发该构件。发布检查和各构件的具体说明见
[`RELEASING.md`](./RELEASING.md)。
