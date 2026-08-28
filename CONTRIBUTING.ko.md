# OpenKache에 기여하기

[English](./CONTRIBUTING.md) | **한국어**

OpenKache는 성능을 최우선으로 하는 캐시 서버이며, 프로젝트를 발전시키는
작업도 그 성격을 반영합니다. 재현 가능한 버그 리포트, 회귀를 짚어내는
벤치마크, 스토리지나 프로토콜 수정, 클라이언트 바인딩, 그리고 사람들이
제대로 운영하도록 돕는 문서까지, 이 모든 기여를 환영합니다.

이 문서는 어디에 질문하고, 문제를 어떻게 보고하며, 변경 사항을 어떻게
리뷰받고 병합하는지를 설명합니다. 프로젝트의 [행동 강령](./CODE_OF_CONDUCT.md)
역할을 하는 [커뮤니티 가이드라인](./COMMUNITY_GUIDELINES.md)도 함께 읽어
주세요.

## 어디에 질문할까

설치, 사용법, 현재 지원 범위는 [README](./README.md),
[시작 가이드](./docs/getting-started.md),
[FAQ](./docs/faq.md)부터 확인하세요. 대부분의 "어떻게 하나요…" 질문은
여기에서 답을 찾을 수 있습니다.

GitHub 이슈는 **재현 가능한 버그와 구체적이고 실행 가능한 기능 요청**을
위한 것이며, 일반적인 질문이나 사용법 문의를 위한 것이 아닙니다. 빈 이슈는
비활성화되어 있으니
[이슈 템플릿](https://github.com/openkache/openkache/issues/new/choose) 중
하나를 사용해 주세요.

## 버그 보고

먼저 [기존 이슈](https://github.com/openkache/openkache/issues)를 검색한 뒤,
문제를 재현하는 가장 작은 사례와 함께
[버그 리포트](https://github.com/openkache/openkache/issues/new/choose)를
여세요. 다음 내용을 포함해 주세요.

- 버전 또는 커밋, 그리고 플랫폼(Linux 커널과 `io_uring` 지원 여부, macOS
  프리뷰, WSL2 등);
- 서버를 시작하고 부하를 주기 위해 사용한 정확한 명령어, 설정, CPU 할당;
- 기대한 동작, 실제 동작, 그리고 서버 로그나 클라이언트 오류.

**성능 리포트는 더 엄격하게 검토합니다.** OpenKache는 구체적인 처리량과
지연 시간 수치를 공개하므로, 회귀나 느린 경로에 대한 리포트에는 측정을
재현할 수 있을 만큼의 정보가 필요합니다. 하드웨어, 부하 도구와 그 파라미터,
키와 값의 크기, 관측한 수치를 함께 적어 주세요. 사용하는 측정 방법론은
[BENCHMARK.md](./BENCHMARK.md)를 참고하세요.

취약점 세부 내용은 공개 이슈에 올리지 마세요.
[GitHub 보안 권고](https://github.com/openkache/openkache/security/advisories/new)를
통해 비공개로 보고해 주세요. 프로젝트의 신뢰 경계는
[SECURITY_MODEL.md](./SECURITY_MODEL.md)에 정리되어 있습니다.

## 변경 제안

**중요한 변경**(새로운 스토리지나 와이어 포맷 동작, 프로토콜 추가, 운영
기본값, 그 밖에 호환성·성능·보안에 영향을 주는 모든 것)은 코드를 작성하기
전에 먼저 이슈를 열어 접근 방식에 합의하세요. 문제가 무엇이고 왜 중요한지
설명해 주세요. 기능이 받아들여지려면 명확한 사용 사례가 있어야 합니다. 이
과정을 거치면 리뷰 단계에서 다시 설계해야 하는 변경을 애초에 피할 수
있습니다.

**작은 문서 수정과 명확하고 범위가 좁은 버그 수정**은 이 단계를 건너뛰고
바로 풀 리퀘스트를 열어도 됩니다.

어느 경우든 리뷰할 수 있을 만큼 변경을 작게 유지하고, 관련 없는 작업은
섞지 마세요. `main`에서 토픽 브랜치를 만들고, 자격 증명, 비공개 데이터,
빌드 산출물, 릴리스 과정에서 생성되는 아티팩트는 커밋하지 마세요.

## 개발 환경 설정

OpenKache는 하나의 lockfile 아래에 있는 Rust 워크스페이스입니다(프로토콜,
서버, 공유 클라이언트 코어, Rust SDK, CLI, 그리고 네이티브 TypeScript
어댑터). 서버 빌드에는 Clang/LLVM 툴체인을 사용합니다. 전체 환경은
[시작 가이드](./docs/getting-started.md)를 참고하세요.

클론당 한 번 git 훅을 설치하세요.

```bash
./scripts/install-hooks.sh
```

`pre-commit` 훅은 3개 국어(영어, 한국어, 중국어) 문서 동기화를 강제합니다.
`README.md`처럼 세 언어로 제공되는 문서를 수정하면, `.ko`와 `.zh` 버전을
맞춰 갱신하지 않는 한 훅이 커밋을 거부합니다. 등록된 문서 세트와 긴급
`--no-verify` 우회 방법은 [scripts/README.md](./scripts/README.md)를
참고하세요.

## 작업 점검

풀 리퀘스트를 열기 전에 저장소 루트에서 다음을 실행하세요.

```bash
cargo fmt --all --check
cargo check --locked
cargo test --locked --package openkache-server
```

TypeScript 클라이언트를 변경했다면 다음도 실행하세요.

```bash
bun install --cwd clients/typescript --frozen-lockfile
bun run --cwd clients/typescript build
```

프로토콜이나 생성된 클라이언트 계약(contract)을 변경했다면, 이를 다시
생성하고 스냅샷이 깨끗한지 확인하세요. CI가 검사하는 것과 동일합니다.

```bash
OPENKACHE_GENERATION_TARGET=rust-snapshots OPENKACHE_GENERATION_CHECK=1 ./clients/generate.ts
```

어떤 검사를 환경에서 실행할 수 없다면, 조용히 건너뛰지 말고 풀 리퀘스트에
그 사실을 적어 주세요. 변경에 맞는 검증을 함께 추가하세요. 특히 버그
수정에는 재현 절차를, 성능 관련 주장에는 측정치를 붙여 주세요.

## 커밋 메시지

무엇이 바뀌었는지 명령형으로 짧게 쓴 제목을 사용하세요. 히스토리는
[Conventional Commits](https://www.conventionalcommits.org/) 접두사(`feat`,
`fix`, `docs`, `ci`, `chore` 등)를 사용하며, 종종 선택적 범위(scope)를
붙입니다. 예: `fix(server): recover the segment index on restart`. 본문에는
이유와 트레이드오프를 적으세요.

## 풀 리퀘스트 열기

병합하기 쉬운 풀 리퀘스트에는 다음이 있습니다.

- 변경 내용을 설명하는 짧은 제목;
- 문제, 의도한 결과, 접근 방식에 대한 설명;
- 관련 이슈나 논의가 있다면 그 링크;
- 실행한 검증 명령어와 그 결과;
- 해당되는 경우 문서, 호환성, 운영 위험, 보안에 대한 참고 사항.

초기 피드백이 재작업을 줄여 줄 때는 드래프트 풀 리퀘스트도 환영합니다.
리뷰는 정확성, 명확한 동작, 안전한 운영, 그리고 변경이 목표한 문제를
실제로 해결하는지에 집중합니다. 리뷰 코멘트는 설계 과정의 일부입니다. 내용에
응답하고, 방향이 바뀌면 설명을 갱신하세요.

## 기여자 라이선스 계약(CLA)

풀 리퀘스트에는 자동 CLA 검사가 실행됩니다. 서명을 요청받으면 기여가
병합되기 전에 안내된 링크를 따라 서명하세요. Apache 기여자 라이선스 계약
템플릿을 사용합니다. 제출할 권리가 있는 작업에 대해서만 서명하고, 조직을
대신해 기여한다면 그럴 권한이 있는지 확인하세요. CLA는 여러분이 변경하는
파일의 라이선스를 대체하지 않으며 함께 적용됩니다.

## 라이선스

기여는 변경 대상 파일에 적용되는 라이선스에 따라 받아들여집니다. 서버는
GNU Affero General Public License v3.0-or-later로 배포됩니다.
[`clients/`](./clients/) 아래의 클라이언트 SDK와 [`protocol/`](./protocol/)
아래의 공유 프로토콜은 각 디렉터리에 명시된 Apache License 2.0을 따릅니다.
기여를 제출함으로써, 해당 라이선스에 따라 제출할 권리가 있음을 확인하는
것입니다.
