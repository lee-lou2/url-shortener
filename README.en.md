# URL Shortener Service

[한국어](README.md) | [English](README.en.md)

🚀 **Demo:** [https://u.lou2.kr](https://u.lou2.kr)

![demo site](docs/screenshot.png)

A high-performance URL shortening service built with Rust. Supports deep link handling, platform-specific redirects, JWT authentication, and webhook notifications.

## Architecture

```mermaid
flowchart TB
    subgraph Client["🌐 Client"]
        Browser[Browser]
        Mobile[Mobile App]
    end

    subgraph Server["⚡ API Server (Axum)"]
        Router[Router]
        Auth[JWT Auth]
        RateLimit[Rate Limiter]
        Handler[Handler]
    end

    subgraph Storage["💾 Storage"]
        Redis[(Redis Cache)]
        PostgreSQL[(PostgreSQL)]
    end

    subgraph External["🔔 External"]
        Webhook[Webhook Endpoint]
    end

    Browser --> Router
    Mobile --> Router
    Router --> RateLimit
    RateLimit --> Auth
    Auth --> Handler
    Handler <--> Redis
    Handler <--> PostgreSQL
    Handler -.->|async| Webhook
```

## Core Technologies

| Area | Technology | Description |
|------|------------|-------------|
| Web Framework | **Axum 0.8** | Async HTTP server |
| Database | **PostgreSQL + SQLx** | Type-safe queries |
| Cache | **Redis + MessagePack** | High-speed serialized caching |
| Auth | **JWT** | Token-based authentication |
| Hashing | **xxHash (xxh3_128)** | Duplicate URL detection |
| Memory | **mimalloc** | High-performance allocator |

## URL Creation Flow

```mermaid
sequenceDiagram
    participant C as Client
    participant S as API Server
    participant R as Redis
    participant DB as PostgreSQL

    C->>S: POST /v1/urls (URL data + JWT)
    S->>S: Validate JWT
    S->>S: Validate input
    S->>S: Generate xxHash (for dedup)
    S->>DB: INSERT ... ON CONFLICT
    
    alt New URL
        DB-->>S: Created URL (id, random_key)
        S->>S: Base62 encode (generate short_key)
    else Existing URL
        DB-->>S: Return existing URL
    end
    
    S-->>C: { short_key: "Ab3D7Xy" }
```

### Short Key Generation

```mermaid
flowchart LR
    subgraph Input["Input"]
        ID["DB ID: 12345"]
        RK["Random Key: AbXy"]
    end

    subgraph Process["Process"]
        B62["Base62 Encode"]
        Split["Split Random Key"]
    end

    subgraph Output["Output"]
        SK["Short Key: Ab3D7Xy"]
    end

    ID --> B62
    B62 --> |"3D7"| Merge
    RK --> Split
    Split --> |"Prefix: Ab"| Merge
    Split --> |"Suffix: Xy"| Merge
    Merge["Merge"] --> SK
```

**Features:**
- No collisions (based on DB ID)
- Random prefix/suffix prevents sequential guessing
- Consistent performance (independent of DB size)

## URL Redirect Flow

```mermaid
sequenceDiagram
    participant C as Client
    participant S as API Server
    participant R as Redis
    participant DB as PostgreSQL
    participant W as Webhook

    C->>S: GET /Ab3D7Xy
    S->>S: Parse short_key (extract id + random_key)
    
    S->>R: GET url:{id}
    alt Cache Hit
        R-->>S: MessagePack data
    else Cache Miss
        R-->>S: null
        S->>DB: SELECT * FROM urls WHERE id = ?
        DB-->>S: URL data
        S->>R: SETEX url:{id} (TTL: 1 hour)
    end

    S->>S: Validate random_key
    S->>S: Detect platform (iOS/Android/Other)
    
    par Async Webhook Call
        S--)W: POST (short_key, user_agent, timestamp)
    end

    S-->>C: HTML (deep link + fallback URL)
```

### Platform-Specific Redirect

```mermaid
flowchart TD
    Request[Request Received] --> Detect{User-Agent Analysis}
    
    Detect -->|iOS| iOS{Deep Link Set?}
    Detect -->|Android| Android{Deep Link Set?}
    Detect -->|Other| Default[Default Fallback URL]
    
    iOS -->|Yes| iOSDeep[Try iOS Deep Link]
    iOS -->|No| iOSFallback[iOS Fallback URL]
    iOSDeep -->|On Failure| iOSFallback
    
    Android -->|Yes| AndroidDeep[Try Android Deep Link]
    Android -->|No| AndroidFallback[Android Fallback URL]
    AndroidDeep -->|On Failure| AndroidFallback

    iOSFallback --> Response[Redirect]
    AndroidFallback --> Response
    Default --> Response
```

## Caching Strategy

```mermaid
flowchart LR
    subgraph Request["Request"]
        R1[URL Lookup]
    end

    subgraph Cache["Redis Cache"]
        Check{Check Cache}
        Hit[Cache Hit]
        Miss[Cache Miss]
        Update[Update Cache]
    end

    subgraph DB["PostgreSQL"]
        Query[DB Query]
    end

    subgraph Serialize["Serialization"]
        MP[MessagePack]
    end

    R1 --> Check
    Check -->|Exists| Hit
    Check -->|Not Found| Miss
    Miss --> Query
    Query --> MP
    MP --> Update
    Update --> Hit
    Hit --> Response[Response]
```

**Why MessagePack:**
- 30-50% smaller than JSON
- Fast serialization/deserialization
- Binary format for efficient Redis storage

## Getting Started

### Prerequisites

- Rust 1.75+
- PostgreSQL
- Redis

### Run

```bash
# Clone repository
git clone https://github.com/lee-lou2/url-shortener.git
cd url-shortener

# Configure environment
cp .env.example .env

# Run
cargo run --release
```

### Docker

```bash
docker build -t url-shortener .
docker run -p 3000:3000 --env-file .env url-shortener
```

### Key Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `SERVER_PORT` | 3000 | Server port |
| `DB_HOST` | localhost | PostgreSQL host |
| `REDIS_HOST` | localhost | Redis host |
| `JWT_SECRET` | - | JWT secret (required in production) |
| `CACHE_TTL_SECS` | 3600 | Cache TTL (seconds) |
| `RATE_LIMIT_PER_SECOND` | 10 | Requests per second limit |
| `WEBHOOK_MAX_CONCURRENT` | 100 | Max concurrent webhooks |

## API

### `POST /v1/urls` - Create URL

**Request:**
```json
{
  "defaultFallbackUrl": "https://example.com",
  "iosDeepLink": "myapp://path",
  "iosFallbackUrl": "https://apps.apple.com/app/myapp",
  "androidDeepLink": "myapp://path",
  "androidFallbackUrl": "https://play.google.com/store/apps/details?id=com.myapp",
  "webhookUrl": "https://webhook.example.com",
  "ogTitle": "Title",
  "ogDescription": "Description",
  "ogImageUrl": "https://example.com/image.jpg"
}
```

**Response:**
```json
{
  "message": "URL created successfully",
  "short_key": "Ab3D7Xy"
}
```

### `GET /{short_key}` - Redirect

Redirects the short URL to the original URL.

## Project Structure

```
src/
├── main.rs           # Entry point
├── error.rs          # Error handling
├── api/              # HTTP handlers, routes, middleware
├── config/           # Environment config, DB/Redis connections
├── models/           # Data models, repositories
└── utils/            # JWT, Base62, random strings
```

## License

MIT License
