# Changelog

## [0.2.0](https://github.com/apestel/omniman/compare/omniman-v0.1.0...omniman-v0.2.0) (2026-05-16)


### Features

* **ai:** configure Gemini API key from settings UI ([b181806](https://github.com/apestel/omniman/commit/b1818068d5a6e4662cc21254d705602ed5ebe9bf))
* **ai:** handle 429 rate limit with countdown UI ([64b1d70](https://github.com/apestel/omniman/commit/64b1d70516fd9d7c5125f6b6cf44c2808ad929a9))
* **ai:** live model list from daemon; fix stale model in requests ([81172d9](https://github.com/apestel/omniman/commit/81172d9e4bf06d5d8dd8bfdb4fd5c4b6e99cd052))
* **ai:** streaming output, pulldown-cmark, OpenAI endpoint ([c2b28ea](https://github.com/apestel/omniman/commit/c2b28eacd4cd51dc28e11d96674f3e4d7f6e8c71))
* **clipboard:** push ClipboardChanged signal on new entry ([a4e5394](https://github.com/apestel/omniman/commit/a4e5394b333fea97d9a4e61faac32ef1e89670db))
* initial implementation of Omniman launcher ([413051e](https://github.com/apestel/omniman/commit/413051eec81bdfcbfb24d8710ae6631494b323d3))


### Bug Fixes

* **ui:** animate AI chunks and hide on focus loss ([3954ac2](https://github.com/apestel/omniman/commit/3954ac2c4b8f36159ebf013424ac46c04a4d43b8))
* **ui:** prevent auto-hide while preferences window is open ([766bdcf](https://github.com/apestel/omniman/commit/766bdcf9a34b667e97df9dbd162ca21db3d2ef50))


### Performance Improvements

* **daemon:** mtime sweep + lower priority to cut CPU/RAM on startup ([74610fa](https://github.com/apestel/omniman/commit/74610fa1f829c63448f5c2e9f39d1d14ba1675b5))
