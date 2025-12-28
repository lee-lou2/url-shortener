# URL 단축 서비스

[한국어](README.md) | [English](README.en.md)

🚀 **데모 사이트:** [https://u.lou2.kr](https://u.lou2.kr)

![demo site](docs/screenshot.png)

## 소개

Rust로 개발된 고성능 URL 단축 서비스입니다. 딥 링크 처리, 플랫폼별 리디렉션, JWT 인증, 웹훅 알림 등 다양한 기능을 제공합니다.

## 주요 기능

| 기능 | 설명 |
|------|------|
| **URL 단축** | Base62 인코딩 기반의 충돌 없는 고유 단축 URL 생성 |
| **딥 링크** | iOS/Android 앱 딥 링크 지원 및 플랫폼별 폴백 URL 처리 |
| **OG 태그** | 소셜 미디어 링크 미리보기를 위한 Open Graph 메타데이터 설정 |
| **웹훅** | URL 접근 시 지정된 엔드포인트로 실시간 알림 전송 (동시성 제어 포함) |
| **Redis 캐싱** | MessagePack 직렬화로 자주 접근되는 URL 정보를 고속 캐싱 |
| **레이트 리미팅** | SmartIP 기반 API 남용 방지를 위한 요청 제한 |
| **다중 압축** | Brotli, Gzip, Zstd 압축으로 응답 크기 최적화 |

## 기술 스택

| 영역 | 기술 |
|------|------|
| 언어 | Rust 2021 Edition |
| 웹 프레임워크 | Axum 0.8 |
| 비동기 런타임 | Tokio |
| 데이터베이스 | PostgreSQL (SQLx) |
| 캐시 | Redis (deadpool-redis) |
| 캐시 직렬화 | MessagePack (rmp-serde) |
| 템플릿 엔진 | Askama |
| 인증 | JWT (jsonwebtoken) |
| 해싱 | xxHash (xxh3_128) |
| 메모리 할당자 | mimalloc |
| 레이트 리미팅 | tower_governor |
| 에러 추적 | Sentry |

## 아키텍처

```
┌─────────────┐     ┌─────────────┐     ┌─────────────┐
│   Client    │────▶│  API Server │────▶│  PostgreSQL │
│  (Browser)  │     │   (Axum)    │     │  (SQLx)     │
└─────────────┘     └──────┬──────┘     └─────────────┘
                          │
                          ▼
                   ┌─────────────┐
                   │    Redis    │
                   │   (Cache)   │
                   └─────────────┘
```

## 단축키 생성 방식

1. **고유 ID 생성**: 데이터베이스에 저장 시 고유한 숫자 ID를 할당
2. **Base62 인코딩**: 숫자 ID를 Base62로 인코딩하여 짧은 문자열로 변환
3. **랜덤 접두사/접미사**: 4자리 랜덤 문자열을 앞뒤 2자씩 배치하여 예측 불가능성 확보

```
예: 랜덤키 "AbXy" → 접두사 "Ab" + ID 12345의 Base62 인코딩 "3D7" + 접미사 "Xy" → "Ab3D7Xy"
```

**장점:**
- 데이터베이스 ID 기반으로 충돌 없음
- 앞뒤 랜덤 문자로 순차적 키 추측 방지 강화
- 데이터베이스 크기와 무관한 일관된 성능

## 시작하기

### 사전 준비

- Rust 1.75 이상
- PostgreSQL
- Redis

### 설치 및 실행

```bash
# 저장소 복제
git clone https://github.com/lee-lou2/url-shortener.git
cd url-shortener

# 환경 변수 설정
cp .env.example .env
# .env 파일 편집

# 실행
cargo run

# 또는 릴리스 빌드
cargo run --release
```

### 환경 변수

```env
# 서버
SERVER_PORT=3000
CORS_ORIGINS=*

# 데이터베이스
DB_HOST=localhost
DB_PORT=5432
DB_USER=postgres
DB_PASSWORD=postgres
DB_NAME=postgres
DB_MAX_CONNECTIONS=20
DB_MIN_CONNECTIONS=2

# Redis
REDIS_HOST=localhost
REDIS_PORT=6379
REDIS_PASSWORD=
CACHE_TTL_SECS=3600

# JWT
JWT_SECRET=your-secret-key
JWT_EXPIRATION_HOURS=24

# 레이트 리미팅
RATE_LIMIT_PER_SECOND=10
RATE_LIMIT_BURST_SIZE=50

# 웹훅
WEBHOOK_TIMEOUT_SECS=10
WEBHOOK_MAX_CONCURRENT=100

# 마이그레이션
RUN_MIGRATIONS=true

# Sentry (선택)
SENTRY_DSN=
SENTRY_TRACES_SAMPLE_RATE=0.1
```

### Docker

```bash
docker build -t url-shortener .
docker run -p 3000:3000 --env-file .env url-shortener
```

## API

### `GET /`
메인 페이지를 렌더링하고 게스트 JWT 토큰을 발급합니다.

### `POST /v1/urls`
새로운 단축 URL을 생성합니다.

**인증:** `Authorization: Bearer <token>` 또는 쿠키

**요청:**
```json
{
  "defaultFallbackUrl": "https://example.com",
  "iosDeepLink": "myapp://path",
  "iosFallbackUrl": "https://apps.apple.com/app/myapp",
  "androidDeepLink": "myapp://path",
  "androidFallbackUrl": "https://play.google.com/store/apps/details?id=com.myapp",
  "webhookUrl": "https://your-server.com/webhook",
  "ogTitle": "제목",
  "ogDescription": "설명",
  "ogImageUrl": "https://example.com/image.jpg"
}
```

**응답:**
```json
{
  "message": "URL created successfully",
  "short_key": "Ab3D7Xy"
}
```

### `GET /{short_key}`
단축 URL을 원본 URL로 리디렉션합니다.
- Redis 캐시 확인 → 캐시 미스 시 DB 조회
- 플랫폼 감지 및 딥 링크/폴백 URL 처리
- 웹훅 비동기 호출 (Semaphore로 동시성 제어)

## 개발

### 빌드 및 테스트

```bash
# 개발 모드
cargo run

# 릴리스 모드
cargo run --release

# 테스트 실행
cargo test

# 테스트 출력 포함
cargo test -- --nocapture

# 린트
cargo clippy

# 코드 포맷팅
cargo fmt
```

### 프로젝트 구조

```
src/
├── main.rs           # 진입점, 서버 부트스트랩
├── lib.rs            # 라이브러리 크레이트
├── error.rs          # 중앙화된 에러 처리
├── api/              # HTTP API 레이어
├── config/           # 환경 설정
├── models/           # 데이터 모델
└── utils/            # 유틸리티 함수

tests/
└── integration_test.rs  # 통합 테스트

views/                # HTML 템플릿 (Askama)
migrations/           # SQL 마이그레이션 (SQLx)
```

## 라이선스

MIT License
