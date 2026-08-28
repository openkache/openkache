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
- [기여하기](#기여하기)
- [라이선스](#라이선스)
- [서드파티 저작권 고지](#서드파티-저작권-고지)

---

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

---

## 아키텍처

<div align="center">

<img src="docs/assets/openkache-architecture.png" alt="OpenKache 아키텍처"/>

</div>

**OpenKache는 왜 빠른가?** 코어를 넘나들지 않기 때문입니다.

대부분의 서버는 스레드 풀이 코어 사이를 오가며 일합니다. 그 대가는 공짜가 아닙니다. 락
경합, 뮤텍스, 컨텍스트 스위치, 그리고 캐시 라인이 코어 사이를 튕겨 다닐 때마다 치르는
동기화·복사 비용. 부하가 높아질수록 바로 이 비용이 처리량을 갉아먹습니다.

OpenKache는 **thread-per-core(shared-nothing)** 설계를 택합니다. 각 워커는 하나의 코어에
고정되고, 자기 데이터만 소유하며, 공유 상태가 없으니 락도 없습니다. 이것은
TigerBeetle·ScyllaDB·Redis가 하드웨어를 끝까지 짜내기 위해 수렴한 바로 그 설계입니다.
네트워크 경로와 스토리지 경로는 각자 코어를 소유하고, 둘 사이는 오직 **락 프리 SPSC 큐**
하나로만 통신합니다. 그래서 RESP 파싱이 디스크 I/O를 막는 일은 결코 없습니다.

Redis는 명령을 단 하나의 코어에서 실행합니다. OpenKache는 같은 shared-nothing 원칙을
코어별 샤딩으로 확장해, 처리량이 단일 코어 천장에 갇히지 않고 하드웨어를 따라 확장됩니다.
공유 락이 없으니 코어를 더해도 경합이 따라붙지 않습니다.

값은 SSD에, 키는 RAM의 압축 인덱스(압축 키 → 세그먼트 오프셋)에 둡니다. 그리고 지하철이
자가용보다 많은 사람을 실어 나르듯, 여러 키의 쓰기를 키마다 하나씩 쓰는 대신 **세그먼트
그룹** 하나의 순차 flush로 묶어 SSD의 순차 대역폭을 끝까지 씁니다. Linux에서는
`io_uring`으로 I/O를 제출해 시스템 콜 오버헤드마저 줄입니다.

이 모든 것을 **Rust**로 씁니다. GC 멈춤 없이, 데이터 경쟁을 컴파일 타임에 배제한 채, C
수준의 제어를 손에 쥐고. 빠른 경로에 GC 일시정지가 끼어들 자리는 없습니다.

전체 설계는 [docs/architecture.md](./docs/architecture.md) 참고. (한국어 번역 준비 중)

---

## 빠른 시작

OpenKache는 **Linux**에 맞춰 최적화하고 벤치마크합니다.

- **Windows:** WSL2 사용을 권장합니다.
- **macOS:** 기능 개발용이며 성능 비교에는 적합하지 않습니다.

Linux 요구사항:

- `io_uring`을 지원하는 Linux
- 프로세스가 쓸 수 있는 서로 다른 CPU 2개

### Docker

이미지를 내려받는다:

```bash
docker pull ghcr.io/openkache/openkache:edge
```

서버를 실행한다:

```bash
docker run --rm \
  --network host \
  --security-opt seccomp=unconfined \
  ghcr.io/openkache/openkache:edge
```

### 내려받아 실행하기

OpenKache는 아직 정식 릴리스 전이다. [server-v0.1.0](https://github.com/openkache/openkache/releases/tag/server-v0.1.0)에서
소스를 내려받아 압축을 풀고 아래 Cargo 명령으로 실행한다.

### Cargo

```bash
cargo run --locked --package openkache-server --bin openkache-server
```

---

## 클라이언트 연결

서버가 실행 중이면, 원하는 언어로 연결합니다. 모든 클라이언트 가이드는 기본 로컬
엔드포인트로 `127.0.0.1:4433`을 사용하며, 각 언어의 전체 공개 API를 안내합니다.

OpenKache는 TypeScript/JavaScript, Python, Rust 클라이언트를 각각 별도 패키지로
제공합니다. 세 패키지는 동일한 프로토콜과 값 형식(value-format) 소스를 공유합니다:

| 패키지 | 설치 | 문서 | 소스 |
|---|---|---|---|
| TypeScript / JavaScript | `npm install openkache` | [npm](https://www.npmjs.com/package/openkache) · [클라이언트 README](clients/typescript/README.md) | [GitHub](https://github.com/openkache/openkache/tree/main/clients/typescript) |
| Python | `python -m pip install openkache` | [PyPI](https://pypi.org/project/openkache/) · [클라이언트 README](clients/python/README.md) | [GitHub](https://github.com/openkache/openkache/tree/main/clients/python) |
| Rust | `cargo add openkache` | [crates.io](https://crates.io/crates/openkache) · [docs.rs](https://docs.rs/openkache/latest/openkache/) · [클라이언트 README](clients/rust/README.md) | [GitHub](https://github.com/openkache/openkache/tree/main/clients/rust) |

.NET, Go, C, C++, Swift 등 추가 바인딩의 상태는
[clients/README.md](./clients/README.md)를 참고하세요.

Rust SDK 한눈에 보기:

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

소스에서 빌드하는 [`openkache-cli`](clients/cli/README.md)는 Bash 친화적 옵션으로,
기본적으로 동일한 고정 Gate 0 프로필을 사용합니다:

```bash
openkache-cli set hello "from cli"
openkache-cli get hello
```

인증서 루트, 상호 TLS, 클라이언트 측 값 보호, 또는 호환성 전용 TTL/조건부 쓰기가 필요한
경우 `openkache-cli --profile configured`를 사용하세요.

---

## 로드맵

| 마일스톤 | 상태 | 초점 |
|---|---|---|
| SSD 스토리지 & 듀얼 프로토콜 서버 | 🚧 진행 중 | 세그먼트 그룹 쓰기, RESP/TCP + QUIC Gate 0 |
| 프로덕션 하드닝 | 🔜 다음 | 재시작 복구, 재현 가능한 벤치마크, 퍼징, CI/CD |
| 보안 & 정확성 | 📅 예정 | 인증/mTLS, 값 보호 프로파일, 전체 프로토콜 |
| 확장 & 저변 | 📅 예정 | 클러스터링, 크로스 플랫폼 서버, 정식 출시(GA) |

상세는 [ROADMAP.md](./ROADMAP.md). (한국어 번역 준비 중)

---

## 기여하기

- [기여 가이드](./CONTRIBUTING.md)
- [커뮤니티 가이드라인](./COMMUNITY_GUIDELINES.md)
- [행동 강령](./CODE_OF_CONDUCT.md)

---

## 라이선스

별도로 명시하지 않은 경우, OpenKache는
[GNU Affero General Public License v3.0 이상](./LICENSE) 하에 라이선스가 부여됩니다.
[`clients/`](./clients/) 아래의 클라이언트 SDK와 [`protocol/`](./protocol/) 아래의 공유
프로토콜은 Apache License 2.0을 따릅니다. 각 디렉터리의 `LICENSE` 파일을 참고하세요.

---

## 서드파티 저작권 고지

릴리스 아카이브, 컨테이너 이미지, 클라이언트 패키지에는 잠긴 Cargo 의존성 그래프에서
생성한 `THIRD-PARTY-NOTICES.txt`가 포함됩니다. 이 파일은 업스트림 라이선스, 고지,
저작자 표시 문구를 보존하며 OpenKache 자체 라이선스는 인접한 `LICENSE` 파일에 있습니다.

별도의 업스트림 라이선스 파일이 없는 항목은 유지관리자 검토 대상으로 표시됩니다. 고지에
`LEGAL REVIEW REQUIRED`가 남아 있는 아티팩트는 재배포하지 마세요. 릴리스 검사와
아티팩트별 지침은 [`RELEASING.md`](./RELEASING.md)를 참고하세요.
