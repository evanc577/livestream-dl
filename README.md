# livestream-dl

An experimental HLS (m3u8) livestream downloader

## Features

- General
  - [x] Download HLS streams
    - [x] Livestreams
    - [x] Non-live videos
    - [x] Also download alternative streams
- Technical
  - [x] Byte-range for URIs
  - [x] Discontinuities
  - [ ] Decryption
    - [x] AES-128
    - [ ] SAMPLE-AES (Usually DRM)
  - [ ] HLS low latency
  - [x] Load cookies from file
- Additional
  - [x] Interactive stream selection
  - [x] Save individual media segments separately
  - [x] Automatically remux into mp4
