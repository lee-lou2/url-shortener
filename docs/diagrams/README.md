# Architecture & Flow Diagrams

Editorial diagrams generated following [`cathrynlavery/diagram-design`](https://github.com/cathrynlavery/diagram-design) conventions (light and dark variants, no drop shadows, 4px grid, orthogonal connectors).

| Diagram | Type | Light PNG | Dark PNG | HTML/SVG Source |
|---------|------|-----------|----------|-----------------|
| System Architecture | Architecture | [architecture.png](architecture.png) | [architecture-dark.png](architecture-dark.png) | [light](src/architecture.html) · [dark](src/architecture-dark.html) |
| URL Creation Flow (`POST /v1/urls`) | Sequence | [url-creation-flow.png](url-creation-flow.png) | [url-creation-flow-dark.png](url-creation-flow-dark.png) | [light](src/url-creation-flow.html) · [dark](src/url-creation-flow-dark.html) |
| Short Key Generation | Pipeline | [short-key-encoding.png](short-key-encoding.png) | [short-key-encoding-dark.png](short-key-encoding-dark.png) | [light](src/short-key-encoding.html) · [dark](src/short-key-encoding-dark.html) |
| URL Redirect Flow (`GET /{short_key}`) | Sequence | [url-redirect-flow.png](url-redirect-flow.png) | [url-redirect-flow-dark.png](url-redirect-flow-dark.png) | [light](src/url-redirect-flow.html) · [dark](src/url-redirect-flow-dark.html) |
| Platform-Specific Redirect | Flowchart | [platform-redirect.png](platform-redirect.png) | [platform-redirect-dark.png](platform-redirect-dark.png) | [light](src/platform-redirect.html) · [dark](src/platform-redirect-dark.html) |
| Two-Tier Cache Strategy | Architecture | [caching-strategy.png](caching-strategy.png) | [caching-strategy-dark.png](caching-strategy-dark.png) | [light](src/caching-strategy.html) · [dark](src/caching-strategy-dark.html) |

Each `.html` file under `src/` is self-contained (inline SVG + embedded CSS) and serves as the source of truth for the `@2` retina `.png` exports embedded in the README and developer guides.
