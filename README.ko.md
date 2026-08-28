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
- [서드파티 저작권 고지](#서드파티-저작권-고지)

## 벤치마크

6 vCPU AMD EPYC 7773X 호스트(SSD, 커널 6.8)에서 루프백 환경으로 측정했으며, 32바이트
키와 100바이트 값을 사용합니다. 각 시스템은 자체 프로토콜로 통신하는 로드 툴로 구동됩니다.
전체 방법론은 [BENCHMARK.md](./BENCHMARK.md)를 참고하세요.

**GET 처리량**

| 시스템 | GET 처리량 | 로드 툴 |
|---|---:|---|
| OpenKache | **97,887 ops/s** | kvbench (RESP) |
| PostgreSQL 17.10 | 17,421 ops/s | kvbench (PostgreSQL wire) |
| MySQL 8.4.11 | 16,295 ops/s | kvbench (MySQL wire) |

OpenKache는 PostgreSQL보다 5.6배, MySQL보다 6.0배 빠르며, 단일 스토리지 코어로 머신의
단일 코어 4 KiB 랜덤 읽기 한계치(128,820 IOPS)의 76%에 도달합니다.

**GET 지연시간 (요청을 한 번에 하나씩 처리)**

| 시스템 | 평균 | p50 | p99 | p99.9 |
|---|---:|---:|---:|---:|
| OpenKache | **238.7 µs** | 229 µs | 386 µs | 1376 µs |
| MySQL 8.4.11 | 385.7 µs | 410 µs | 1169 µs | 2207 µs |
| PostgreSQL 17.10 | 558.0 µs | 510 µs | 1263 µs | 3342 µs |

평균 GET 지연시간은 MySQL보다 1.6배, PostgreSQL보다 2.3배 낮습니다. p99 기준으로는
각각 3.0배, 3.3배 낮습니다.

## 아키텍처

<div align="center">

<img src="docs/assets/openkache-architecture.png" alt="OpenKache 아키텍처"/>

</div>

> **모든 cache는 RAM에 걸었다. disk가 느렸으니까.**
> **SSD가 판을 바꿨다. 느린 건 이제 disk가 아니다. CPU다.**
> **그래서 OpenKache는 SSD-first로 짰다.**

값은 SSD에 두고, RAM에는 작은 index만 남긴다. 여기서부터 모든 설계 결정은 CPU가 노는 순간을
하나씩 지워 나간다.

**공유하는 데이터가 없으니 기다릴 일도 없다.** 보통 database는 하나의 데이터를 여러 core가
같이 쓰고, 서로 망가뜨리지 못하게 lock을 건다. 그러다 보니 core들이 일은 안 하고 lock 풀리기만
기다린다. OpenKache는 core마다 자기 몫의 데이터를 따로 준다. 같이 쓰는 게 없으니 lock 걸 일도
없고, 한 core가 다른 core를 기다리지도 않는다. (이걸 shared-nothing 설계라고 한다. [KVell,
SOSP '19](https://dl.acm.org/doi/10.1145/3341301.3359628))

**읽기랑 쓰기가 서로를 안 막는다.** 한 core는 network를 읽어서 요청을 해석하고, 다른 core는
disk를 맡는다. 둘은 한 방향 통로로 일을 넘긴다. 앞 core가 요청을 놓아두면 뒤 core가 집어간다.
같은 걸 동시에 건드릴 일이 없으니, 서로 멈춰서 기다릴 필요가 없다.

**찔끔찔끔 말고 한 번에 몰아서 쓴다.** SSD에 작은 데이터를 여기저기 흩뿌리며 쓰는 게 제일 느리고,
드라이브 수명도 그만큼 깎인다. OpenKache는 쓸 것들을 모아서 한 번에 쭉 이어 쓴다. 그래서
드라이브가 제 속도를 내고 더 오래 간다. (이걸 segment-group batching이라고 한다. [FairyWren,
OSDI '24](https://www.usenix.org/conference/osdi24/presentation/mcallister))

**Rust로 짰다.** GC가 없어서 요청 처리 중에 서버가 갑자기 멈추는 일이 없다. 게다가 동시성 버그
상당수를 compile 단계에서 잡아낸다. production까지 갈 일이 없다.

전체 설계는 [docs/architecture.ko.md](./docs/architecture.ko.md)에 있다.

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
brew tap-new openkache/local
install -m 0644 openkache.rb "$(brew --repository openkache/local)/Formula/openkache.rb"
brew install openkache/local/openkache
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

## 서드파티 저작권 고지

릴리스 아카이브, 컨테이너 이미지, 클라이언트 패키지에는 잠긴 Cargo 의존성 그래프에서
생성한 `THIRD-PARTY-NOTICES.txt`가 포함됩니다. 이 파일은 업스트림 라이선스, 고지,
저작자 표시 문구를 보존하며 OpenKache 자체 라이선스는 인접한 `LICENSE` 파일에 있습니다.

별도의 업스트림 라이선스 파일이 없는 항목은 유지관리자 검토 대상으로 표시됩니다. 고지에
`LEGAL REVIEW REQUIRED`가 남아 있는 아티팩트는 재배포하지 마세요. 릴리스 검사와
아티팩트별 지침은 [`RELEASING.md`](./RELEASING.md)를 참고하세요.
