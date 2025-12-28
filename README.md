# URL 단축 서비스

[한국어](README.md) | [English](README.en.md)

🚀 **데모:** [https://u.lou2.kr](https://u.lou2.kr)

![demo site](docs/screenshot.png)

Rust로 개발된 고성능 URL 단축 서비스입니다. 딥 링크 처리, 플랫폼별 리디렉션, JWT 인증, 웹훅 알림을 지원합니다.

## 아키텍처

```mermaid
flowchart TB
    subgraph Client["🌐 클라이언트"]
        Browser[브라우저]
        Mobile[모바일 앱]
    end

    subgraph Server["⚡ API 서버 (Axum)"]
        Router[라우터]
        Auth[JWT 인증]
        RateLimit[레이트 리미터]
        Handler[핸들러]
    end

    subgraph Storage["💾 저장소"]
        Redis[(Redis Cache)]
        PostgreSQL[(PostgreSQL)]
    end

    subgraph External["🔔 외부"]
        Webhook[웹훅 엔드포인트]
    end

    Browser --> Router
    Mobile --> Router
    Router --> RateLimit
    RateLimit --> Auth
    Auth --> Handler
    Handler <--> Redis
    Handler <--> PostgreSQL
    Handler -.->|비동기| Webhook
```

## 핵심 기술

| 영역 | 기술 | 설명 |
|------|------|------|
| 웹 프레임워크 | **Axum 0.8** | 비동기 HTTP 서버 |
| 데이터베이스 | **PostgreSQL + SQLx** | 타입 안전 쿼리 |
| 캐시 | **Redis + MessagePack** | 고속 직렬화 캐싱 |
| 인증 | **JWT** | 토큰 기반 인증 |
| 해싱 | **xxHash (xxh3_128)** | 중복 URL 감지 |
| 메모리 | **mimalloc** | 고성능 할당자 |

## URL 생성 플로우

```mermaid
sequenceDiagram
    participant C as 클라이언트
    participant S as API 서버
    participant R as Redis
    participant DB as PostgreSQL

    C->>S: POST /v1/urls (URL 데이터 + JWT)
    S->>S: JWT 검증
    S->>S: 입력값 유효성 검사
    S->>S: xxHash 생성 (중복 체크용)
    S->>DB: INSERT ... ON CONFLICT
    
    alt 새 URL
        DB-->>S: 생성된 URL (id, random_key)
        S->>S: Base62 인코딩 (short_key 생성)
    else 기존 URL
        DB-->>S: 기존 URL 반환
    end
    
    S-->>C: { short_key: "Ab3D7Xy" }
```

### 단축키 생성 방식

```mermaid
flowchart LR
    subgraph Input["입력"]
        ID["DB ID: 12345"]
        RK["랜덤키: AbXy"]
    end

    subgraph Process["처리"]
        B62["Base62 인코딩"]
        Split["랜덤키 분리"]
    end

    subgraph Output["출력"]
        SK["단축키: Ab3D7Xy"]
    end

    ID --> B62
    B62 --> |"3D7"| Merge
    RK --> Split
    Split --> |"접두사: Ab"| Merge
    Split --> |"접미사: Xy"| Merge
    Merge["결합"] --> SK
```

**특징:**
- DB ID 기반으로 충돌 없음
- 랜덤 접두사/접미사로 순차 추측 방지
- 일관된 성능 (DB 크기 무관)

## URL 리디렉션 플로우

```mermaid
sequenceDiagram
    participant C as 클라이언트
    participant S as API 서버
    participant R as Redis
    participant DB as PostgreSQL
    participant W as 웹훅

    C->>S: GET /Ab3D7Xy
    S->>S: short_key 파싱 (id + random_key 추출)
    
    S->>R: GET url:{id}
    alt 캐시 히트
        R-->>S: MessagePack 데이터
    else 캐시 미스
        R-->>S: null
        S->>DB: SELECT * FROM urls WHERE id = ?
        DB-->>S: URL 데이터
        S->>R: SETEX url:{id} (TTL: 1시간)
    end

    S->>S: random_key 검증
    S->>S: 플랫폼 감지 (iOS/Android/기타)
    
    par 비동기 웹훅 호출
        S--)W: POST (short_key, user_agent, timestamp)
    end

    S-->>C: HTML (딥링크 + 폴백 URL)
```

### 플랫폼별 리디렉션

```mermaid
flowchart TD
    Request[요청 수신] --> Detect{User-Agent 분석}
    
    Detect -->|iOS| iOS{딥링크 설정?}
    Detect -->|Android| Android{딥링크 설정?}
    Detect -->|기타| Default[기본 폴백 URL]
    
    iOS -->|있음| iOSDeep[iOS 딥링크 시도]
    iOS -->|없음| iOSFallback[iOS 폴백 URL]
    iOSDeep -->|실패시| iOSFallback
    
    Android -->|있음| AndroidDeep[Android 딥링크 시도]
    Android -->|없음| AndroidFallback[Android 폴백 URL]
    AndroidDeep -->|실패시| AndroidFallback

    iOSFallback --> Response[리디렉션]
    AndroidFallback --> Response
    Default --> Response
```

## 캐싱 전략

```mermaid
flowchart LR
    subgraph Request["요청"]
        R1[URL 조회]
    end

    subgraph Cache["Redis 캐시"]
        Check{캐시 확인}
        Hit[캐시 히트]
        Miss[캐시 미스]
        Update[캐시 갱신]
    end

    subgraph DB["PostgreSQL"]
        Query[DB 조회]
    end

    subgraph Serialize["직렬화"]
        MP[MessagePack]
    end

    R1 --> Check
    Check -->|존재| Hit
    Check -->|없음| Miss
    Miss --> Query
    Query --> MP
    MP --> Update
    Update --> Hit
    Hit --> Response[응답]
```

**MessagePack 사용 이유:**
- JSON 대비 30-50% 작은 크기
- 빠른 직렬화/역직렬화
- 바이너리 포맷으로 Redis 저장 효율적

## 시작하기

### 사전 준비

- Rust 1.75+
- PostgreSQL
- Redis

### 실행

```bash
# 저장소 복제
git clone https://github.com/lee-lou2/url-shortener.git
cd url-shortener

# 환경 변수 설정
cp .env.example .env

# 실행
cargo run --release
```

### Docker

```bash
docker build -t url-shortener .
docker run -p 3000:3000 --env-file .env url-shortener
```

### 주요 환경 변수

| 변수 | 기본값 | 설명 |
|------|--------|------|
| `SERVER_PORT` | 3000 | 서버 포트 |
| `DB_HOST` | localhost | PostgreSQL 호스트 |
| `REDIS_HOST` | localhost | Redis 호스트 |
| `JWT_SECRET` | - | JWT 시크릿 (프로덕션 필수) |
| `CACHE_TTL_SECS` | 3600 | 캐시 TTL (초) |
| `RATE_LIMIT_PER_SECOND` | 10 | 초당 요청 제한 |
| `WEBHOOK_MAX_CONCURRENT` | 100 | 최대 동시 웹훅 수 |

## API

### `POST /v1/urls` - URL 생성

**요청:**
```json
{
  "defaultFallbackUrl": "https://example.com",
  "iosDeepLink": "myapp://path",
  "iosFallbackUrl": "https://apps.apple.com/app/myapp",
  "androidDeepLink": "myapp://path",
  "androidFallbackUrl": "https://play.google.com/store/apps/details?id=com.myapp",
  "webhookUrl": "https://webhook.example.com",
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

### `GET /{short_key}` - 리디렉션

단축 URL을 원본 URL로 리디렉션합니다.

## 프로젝트 구조

```
src/
├── main.rs           # 진입점
├── error.rs          # 에러 처리
├── api/              # HTTP 핸들러, 라우트, 미들웨어
├── config/           # 환경 설정, DB/Redis 연결
├── models/           # 데이터 모델, 리포지토리
└── utils/            # JWT, Base62, 랜덤 문자열
```

## 라이선스

MIT License
