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
- [로드맵](#로드맵)
- [기여하기](#기여하기)
- [라이선스](#라이선스)
- [서드파티 저작권 고지](#서드파티-저작권-고지)

---

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

---

## 아키텍처

<div align="center">

<img src="docs/assets/openkache-architecture.png" alt="OpenKache 아키텍처"/>

</div>

모든 요청은 짧고 예측 가능한 한 경로를 따릅니다.

1. 클라이언트가 RESP/TCP 또는 OpenKache/QUIC으로 `GET`, `SET`, `DELETE`를 보냅니다.
2. 하나의 코어에 고정된 네트워크 워커가 스토리지에 접근하지 않고 요청을 파싱합니다.
3. 요청은 하나의 락 프리 **SPSC(single-producer, single-consumer)** 큐를 지나 스토리지
   워커로 전달됩니다. 스토리지 워커도 하나의 코어에 고정됩니다.
4. 스토리지 워커는 작은 RAM 테이블로 후보 4 KiB 버킷을 고른 뒤, 버킷 안의 32바이트
   스토리지 키를 확인합니다. 변경 가능한 버킷은 이미 RAM에 있고, flush된 버킷은 SSD에서
   읽습니다.

### 빠른 이유

- **핫 상태가 한 코어에 머뭅니다.** 각 워커는 자기 데이터를 소유하고 한 코어에 고정되어
  공유 락과 캐시 라인 이동을 줄입니다. 최신 NVMe 장치를 포화시키기 위해 shared-nothing
  원칙을 사용한 [SOSP '19 · KVell](https://github.com/BLepers/KVell/blob/master/sosp19-final40.pdf)과
  같은 방향입니다.
- **RAM은 키가 아니라 위치 단서만 저장합니다.** 조회 테이블에는 8비트 fingerprint와 작은
  세그먼트/버킷 후보만 남습니다. 후보 버킷이 값 옆의 전체 32바이트 스토리지 키를 확인하므로,
  RAM은 모든 키의 복사본 대신 작은 라우팅 메타데이터에 쓰입니다
  ([SIGMOD '26 · Breadcrumb Filters](https://doi.org/10.1145/3786629)).
- **작은 쓰기를 순차 배치로 바꿉니다.** 세그먼트 그룹은 요청마다 SSD 쓰기를 하나씩
  발행하지 않고 여러 키의 쓰기를 더 큰 flush 하나로 합칩니다. 하이브리드 플래시 캐시도
  같은 방식으로 작은 객체의 쓰기를 큰 순차 I/O로 바꿉니다
  ([NSDI '19 · Flashield](https://www.usenix.org/conference/nsdi19/presentation/eisenman),
  [ASPLOS '26 · Nemo](https://github.com/XMU-DISCLab/Cachelib-Nemo)).
- **Linux에서는 `io_uring`을 사용합니다.** 네트워크 워커가 요청을 계속 처리하는 동안
  OpenKache는 비동기 I/O 제출과 완료를 묶어 처리합니다.

서버는 Rust로 작성되어 요청 경로의 메모리 소유권을 명시적으로 관리하고 가비지 컬렉터를
사용하지 않습니다.

전체 설계는 [docs/architecture.md](./docs/architecture.md) 참고. (한국어 번역 준비 중)

---

## 빠른 시작

설치 프로그램은 최신 태그 릴리스를 내려받고 Linux x86-64, Linux ARM64 또는 Apple
Silicon macOS에 맞는 아카이브를 선택한 뒤 SHA-256 체크섬을 검증합니다. 그런 다음
`openkache-server`와 `openkache-cli`를 모두 `~/.local/bin`에 설치합니다. Windows에서는
WSL2에서 Linux 릴리스를 실행할 수 있습니다.

### OpenKache 설치

```bash
curl -fsSL https://github.com/openkache/openkache/raw/main/install.sh | sh
```

[설치 프로그램](./install.sh)의 내용을 먼저 확인할 수도 있습니다.

셸에서 `command not found`가 출력되면 현재 터미널의 `PATH`에 설치 디렉터리를
추가합니다.

```bash
export PATH="$HOME/.local/bin:$PATH"
```

Linux에서는 `io_uring`과 사용 가능한 CPU 2개가 필요합니다. Apple Silicon macOS
바이너리는 기능 개발용 네이티브 경로를 사용하며, 성능 수치는 Linux에만 적용됩니다.

### 터미널 1: 서버 시작

```bash
openkache-server
```

OpenKache를 사용하는 동안 이 터미널을 열어 둡니다.

### 터미널 2: 서버 확인

두 번째 터미널을 열고 실행합니다.

```bash
openkache-cli ping
# PONG

openkache-cli set hello "from CLI"
# CREATED

openkache-cli get hello
# from CLI
```

서버를 중지하려면 터미널 1로 돌아가 <kbd>Ctrl</kbd>+<kbd>C</kbd>를 누릅니다.

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

각 태그 서버 릴리스에 포함된 [`openkache-cli`](clients/cli/README.md) 바이너리는 Bash
친화적 옵션입니다. 기본적으로 동일한 고정 Gate 0 프로필을 사용하며, 전체 예시는
[빠른 시작](#빠른-시작)을 참고하세요.

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
