<div align="center">

# OpenKache ⚡

**OpenKache is a high-performance cache server designed from the ground up for modern SSDs.**

오픈소스 · RESP/TCP · OpenKache/QUIC · Linux `io_uring`

[![Build](https://img.shields.io/badge/build-preview-orange.svg)](https://github.com/openkache/openkache/actions)
[![Rust](https://img.shields.io/badge/rust-2024-orange.svg)](https://www.rust-lang.org/)

[English](./README.md) · **한국어** · [中文](./README.zh.md)

</div>

## 목차

- [벤치마크](#벤치마크)
- [아키텍처](#아키텍처)
- [빠른 시작](#빠른-시작)
- [클라이언트 연결](#클라이언트-연결)
- [컨테이너 이미지](#컨테이너-이미지)
- [로드맵](#로드맵)
- [빌드 및 검증](#빌드-및-검증)
- [클라이언트 패키지](#클라이언트-패키지)
- [저장소 구조](#저장소-구조)
- [프로젝트 상태](#프로젝트-상태)
- [기여하기](#기여하기)
- [라이선스](#라이선스)

## 벤치마크

벤치마크는 [serveroptima1](./benchmark/BENCHMARK.md#test-environment)의 loopback에서
실행했다.

세 시스템 모두 각자의 native protocol로 kvbench 벤치마크를 실행했다.
데이터베이스마다 프로토콜이 달라서, 같은 기준으로 측정할 수 있는 kvbench를 만들었다.
전체 방법론과 kvbench 설명은 [benchmark/BENCHMARK.md](./benchmark/BENCHMARK.md)에 정리했다.

**GET 처리량**

| 시스템 | GET 처리량 | 로드 툴 |
|---|---:|---|
| OpenKache | **97,887 ops/s (1×)** | kvbench (RESP) |
| PostgreSQL 17.10 | 17,421 ops/s (0.18×) | kvbench (PostgreSQL wire) |
| MySQL 8.4.11 | 16,295 ops/s (0.17×) | kvbench (MySQL wire) |

OpenKache는 하드웨어 한계값의 76%([fio](https://github.com/axboe/fio)로
측정한 128,820 IOPS)에 도달한다.

**GET 지연시간 (요청을 한 번에 하나씩 처리)**

| 시스템 | 평균 | p50 | p99 | p99.9 |
|---|---:|---:|---:|---:|
| OpenKache | **238.7 µs (1×)** | 229 µs (1×) | 386 µs (1×) | 1376 µs (1×) |
| MySQL 8.4.11 | 385.7 µs (1.6×) | 410 µs (1.8×) | 1169 µs (3.0×) | 2207 µs (1.6×) |
| PostgreSQL 17.10 | 558.0 µs (2.3×) | 510 µs (2.2×) | 1263 µs (3.3×) | 3342 µs (2.4×) |

## 아키텍처

<div align="center">

<img src="docs/assets/openkache-architecture.png" alt="OpenKache 아키텍처"/>

</div>

**OpenKache는 왜 빠른가?** 코어를 넘나들지 않기 때문입니다.

대부분의 서버는 스레드 풀이 코어 사이를 오가며 일합니다. 그 대가는 공짜가 아닙니다 — 락
경합, 뮤텍스, 컨텍스트 스위치, 그리고 캐시 라인이 코어 사이를 튕겨 다닐 때마다 치르는
동기화·복사 비용. 부하가 높아질수록 바로 이 비용이 처리량을 갉아먹습니다.

OpenKache는 **thread-per-core(shared-nothing)** 설계를 택합니다. 각 워커는 하나의 코어에
고정되고, 자기 데이터만 소유하며, 공유 상태가 없으니 락도 없습니다. 이것은
TigerBeetle·ScyllaDB·Redis가 하드웨어를 끝까지 짜내기 위해 수렴한 바로 그 설계입니다.
네트워크 경로와 스토리지 경로는 각자 코어를 소유하고, 둘 사이는 오직 **락 프리 SPSC 큐**
하나로만 통신합니다. 그래서 RESP 파싱이 디스크 I/O를 막는 일은 결코 없습니다.

Redis는 명령을 단 하나의 코어에서 실행합니다. OpenKache는 같은 shared-nothing 원칙을
코어별 샤딩으로 확장해, 처리량이 단일 코어 천장에 갇히지 않고 하드웨어를 따라 확장됩니다 —
공유 락이 없으니 코어를 더해도 경합이 따라붙지 않습니다.

값은 SSD에, 키는 RAM의 압축 인덱스(압축 키 → 세그먼트 오프셋)에 둡니다. 그리고 지하철이
자가용보다 많은 사람을 실어 나르듯, 여러 키의 쓰기를 키마다 하나씩 쓰는 대신 **세그먼트
그룹** 하나의 순차 flush로 묶어 SSD의 순차 대역폭을 끝까지 씁니다 — Linux에서는
`io_uring`으로 시스템 콜마저 지워가며.

이 모든 것을 **Rust**로 씁니다. GC 멈춤 없이, 데이터 경쟁을 컴파일 타임에 배제한 채, C
수준의 제어를 손에 쥐고. 빠른 경로에 GC 일시정지가 끼어들 자리는 없습니다.

전체 설계는 [docs/architecture.md](./docs/architecture.md) 참고. (한국어 번역 준비 중)

## 빠른 시작

OpenKache는 **Linux**에 맞춰 최적화하고 벤치마크합니다. 고성능 `io_uring` 네트워크
프론트엔드, 다이렉트 I/O 스토리지 경로, CPU 고정 런타임은 Linux 전용입니다. Apple
Silicon macOS에는 Tokio polling과 buffered file I/O를 사용하는 의도적으로 비최적화된
이식성 프리뷰가 있으며, 성능 비교가 아닌 기능 개발용입니다. Windows 네이티브 서버는
없고, WSL2는 `io_uring`을 허용하는 커널에서 Linux 빌드를 사용합니다.

클라이언트를 연결하기 전에 먼저 서버를 실행하세요. 환경에 맞는 첫 번째 방법을 선택합니다:
컨테이너 이미지, Homebrew 또는 APT 패키지, 릴리스 아카이브, Cargo 순입니다.

Linux 요구사항:

- `io_uring`을 지원하는 Linux
- 프로세스가 쓸 수 있는 서로 다른 CPU 2개

### 컨테이너 이미지

GHCR 인증 없이 게시된 프리뷰 이미지를 실행합니다:

```bash
podman run --rm \
  --security-opt seccomp=unconfined \
  --publish 4433:4433/tcp \
  --publish 4433:4433/udp \
  ghcr.io/openkache/openkache:edge
```

`edge` 태그는 `main` 브랜치의 가장 최근 성공한 빌드를 따라갑니다. 재현 가능한 배포가
필요하다면 계속 바뀌는 태그 대신 멀티플랫폼 매니페스트를 `sha256` 다이제스트로 고정하세요.

이미지를 로컬에서 빌드하려면 저장소 루트에서 실행합니다:

```bash
docker build --file server/Dockerfile --tag localhost/openkache:dev .
docker run --rm \
  --security-opt seccomp=unconfined \
  --publish 4433:4433/tcp \
  --publish 4433:4433/udp \
  --volume openkache-data:/var/lib/openkache \
  localhost/openkache:dev
```

기본 컨테이너 커맨드는 네트워크 스레드를 CPU 0번에, 스토리지 스레드를 CPU 1번에
고정합니다. 컨테이너의 CPU 세트가 다른 ID를 사용한다면 커맨드를 재정의하세요. 자세한
내용은 [컨테이너 가이드](./docs/container-image.md)를 참고하세요.

### Homebrew (Apple Silicon macOS)

해당 릴리스에 첨부된 formula를 내려받아 Homebrew로 서버와 CLI를 설치합니다. Formula
테스트는 서버를 시작한 뒤 `openkache-cli`로 `PING`, `SET`, `GET`, `DELETE`를 실행합니다.

```bash
VERSION="${SERVER_VERSION:-0.1.0}"
BASE="https://github.com/openkache/openkache/releases/download/server-v${VERSION}"
curl --fail --location --remote-name "${BASE}/openkache.rb"
brew install --formula ./openkache.rb
openkache-server
```

macOS 서버는 의도적으로 최적화하지 않았습니다. 로컬 기능 개발을 위한 프로토콜 계약은
유지하지만, 게시된 모든 성능 수치는 Linux 서버를 기준으로 합니다.

### APT 패키지 (Ubuntu, Debian, WSL2)

머신의 Debian 아키텍처에 맞는 패키지를 내려받아 APT로 설치합니다. 패키지에는 서버,
CLI, 설정 파일, loopback 전용 systemd unit이 들어 있으며 서비스는 자동 활성화하지
않습니다.

```bash
VERSION="${SERVER_VERSION:-0.1.0}"
ARCH="$(dpkg --print-architecture)"
BASE="https://github.com/openkache/openkache/releases/download/server-v${VERSION}"
PACKAGE="openkache_${VERSION}_${ARCH}.deb"
curl --fail --location --remote-name "${BASE}/${PACKAGE}"
sudo apt install "./${PACKAGE}"
openkache-server
```

systemd 호스트에서는 `sudo systemctl enable --now openkache`로 선택적 서비스를 시작할 수
있습니다. systemd가 없는 WSL2에서는 `openkache-server`를 foreground로 실행합니다.

### Linux 릴리스 아카이브

`server-v<version>` 릴리스가 게시되면 [GitHub Releases](https://github.com/openkache/openkache/releases)에서
Linux 환경에 맞는 아카이브를 다운로드합니다. `SERVER_VERSION`을 게시된 버전으로 지정한
후 아카이브를 검증하고 실행하세요:

```bash
VERSION="${SERVER_VERSION:-0.1.0}"
PLATFORM="linux-x86_64-musl" # arm64에서는 linux-aarch64-musl 사용
BASE="https://github.com/openkache/openkache/releases/download/server-v${VERSION}"
ARCHIVE="openkache-server-${VERSION}-${PLATFORM}.tar.gz"
curl --fail --location --remote-name "${BASE}/${ARCHIVE}"
curl --fail --location --remote-name "${BASE}/SHA256SUMS"
grep -F " ${ARCHIVE}" SHA256SUMS | sha256sum --check
tar -xzf "${ARCHIVE}"
"./openkache-server-${VERSION}-${PLATFORM}/openkache-server"
```

### Cargo

소스에서 빌드하려면 Rust와 워크스페이스 의존성이 요구하는 네이티브 툴체인도 필요합니다.
CPU 0번과 1번을 사용해 서버를 `127.0.0.1:4433`에서 실행합니다:

```bash
cargo run --locked --package openkache-server --bin openkache-server
```

서버는 RESP/TCP와 OpenKache/QUIC에 동일한 숫자 주소를 사용합니다. 다른 주소와 CPU 쌍을
선택하려면:

```bash
cargo run --locked --package openkache-server --bin openkache-server -- \
  0.0.0.0:4433 2 3
```

캐시 파일은 프로세스 작업 디렉터리에 생성되며 서버가 시작될 때마다 초기화됩니다.

## 클라이언트 연결

서버가 실행 중이면, 원하는 언어로 연결합니다. 모든 클라이언트 가이드는 기본 로컬
엔드포인트로 `127.0.0.1:4433`을 사용하며, 각 언어의 전체 공개 API를 안내합니다.

| 패키지 | 설치 | 문서 | 소스 |
|---|---|---|---|
| TypeScript / JavaScript | `npm install openkache` | [npm](https://www.npmjs.com/package/openkache) · [클라이언트 README](clients/typescript/README.md) | [GitHub](https://github.com/openkache/openkache/tree/main/clients/typescript) |
| Python | `python -m pip install openkache` | [PyPI](https://pypi.org/project/openkache/) · [클라이언트 README](clients/python/README.md) | [GitHub](https://github.com/openkache/openkache/tree/main/clients/python) |
| Rust | `cargo add openkache` | [crates.io](https://crates.io/crates/openkache) · [docs.rs](https://docs.rs/openkache/latest/openkache/) · [클라이언트 README](clients/rust/README.md) | [GitHub](https://github.com/openkache/openkache/tree/main/clients/rust) |

Rust SDK 한눈에 보기:

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

소스에서 빌드하는 [`openkache-cli`](clients/cli/README.md)는 Bash 친화적 옵션으로,
기본적으로 동일한 고정 Gate 0 프로필을 사용합니다:

```bash
openkache-cli set hello "from cli"
openkache-cli get hello
```

인증서 루트, 상호 TLS, 클라이언트 측 값 보호, 또는 호환성 전용 TTL/조건부 쓰기가 필요한
경우 `openkache-cli --profile configured`를 사용하세요.

> **로컬 개발용 신뢰 프로필.** 기본 Gate 0 프로필은 로컬 개발용입니다: QUIC 위에서 TLS
> 1.3을 사용하고 평문으로 폴백하지 않지만, 서버 인증서를 검증하지 않습니다. 이 신뢰
> 프로필을 운영(production) 트래픽에 재사용하지 마세요.

## 로드맵

| 마일스톤 | 상태 | 초점 |
|---|---|---|
| SSD 스토리지 & 듀얼 프로토콜 서버 | 🚧 진행 중 | 세그먼트 그룹 쓰기, RESP/TCP + QUIC Gate 0 |
| 프로덕션 하드닝 | 🔜 다음 | 재시작 복구, 재현 가능한 벤치마크, 퍼징, CI/CD |
| 보안 & 정확성 | 📅 예정 | 인증/mTLS, 값 보호 프로파일, 전체 프로토콜 |
| 확장 & 저변 | 📅 예정 | 클러스터링, 크로스 플랫폼 서버, 정식 출시(GA) |

상세는 [ROADMAP.md](./ROADMAP.md). (한국어 번역 준비 중)

## 빌드 및 검증

```bash
cargo check --locked
cargo test --locked --package openkache-server
cargo server-build
```

루트 Cargo 워크스페이스는 프로토콜, 서버, 공유 클라이언트 코어, Rust SDK, CLI, 네이티브
TypeScript 어댑터를 하나의 락파일 아래에서 관리합니다.

서버 할당자(allocator) 실험은 옵트인 기능으로 제공됩니다:

```bash
cargo server-build --features alloc-jemalloc
cargo server-build --features alloc-mimalloc
```

두 할당자 기능을 동시에 활성화하지 마세요.

## 클라이언트 패키지

유지 관리되는 클라이언트 패키지들은 동일한 프로토콜과 값 형식(value-format) 소스를
공유합니다. Rust, TypeScript, Python, .NET, Go, C, C++, Swift 등 각 바인딩의 현재 상태는
[clients/README.md](./clients/README.md)를 참고하세요.

현재 서버의 호환성 프론트엔드는 위에 나열된 Gate 0 오퍼레이션 하위 집합만 지원합니다.
타깃 계약(target contract)에 기술된 더 넓은 API가 서버가 실제로 구현하기 전에 생성된
클라이언트에 먼저 존재할 수 있습니다.

## 저장소 구조

| 경로 | 내용 |
| --- | --- |
| `server/` | 현재 SSD 캐시 서버와 컨테이너 정의 |
| `protocol/` | 공유 와이어 모델, 생성된 계약, 코덱 |
| `clients/` | 클라이언트 SDK와 네이티브 어댑터 |
| `docs/` | 현재 사용 가이드와 명시적으로 식별된 목표 문서 |

현재 서버 구현은 [server/README.md](./server/README.md)에 있습니다. 프로토콜 세부사항은
[protocol/README.md](./protocol/README.md)에 있습니다.

## 프로젝트 상태

| 구성 요소 | 상태 |
| --- | --- |
| RESP/TCP 서버 | 프리뷰 |
| OpenKache/QUIC Gate 0 서버 | 프리뷰 |
| SSD 스토리지 및 삭제 | 프리뷰 |
| 재시작 복구 | 미구현 |
| 운영 환경 인증 | 미구현 |
| 클라이언트 SDK | 프리뷰; 패키지별 상태 참고 |
| 컨테이너 이미지 | Linux amd64/arm64 지원 |
| 클러스터링 | 시작 전 |

## 기여하기

- [기여 가이드](./CONTRIBUTING.md)
- [커뮤니티 가이드라인](./COMMUNITY_GUIDELINES.md)
- [행동 강령](./CODE_OF_CONDUCT.md)

## 라이선스

별도로 명시하지 않은 경우, OpenKache는
[GNU Affero General Public License v3.0 이상](./LICENSE) 하에 라이선스가 부여됩니다.
[`clients/`](./clients/) 아래의 클라이언트 SDK와 [`protocol/`](./protocol/) 아래의 공유
프로토콜은 Apache License 2.0을 따릅니다. 각 디렉터리의 `LICENSE` 파일을 참고하세요.
