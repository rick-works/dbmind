# 第三方组件与许可声明（Third-Party Notices）

DBMind 数据库管理工具软件 V2.0.25
Copyright 2026 ZhongtangLi

本文件随软件分发，用于履行 Apache License 2.0 第 4 条以及其它第三方许可的声明义务。
DBMind 本体以 Apache License 2.0 发布，条款见随包的 LICENSE，版权与商标声明见 NOTICE。

清单由 `scripts\build\third-party-notices.ps1` 从 Cargo.lock、node_modules 与随包 jar
中自动汇总；各组件许可全文可从其源码仓库或 https://spdx.org/licenses/ 取得。

## 1. 随包分发的 Java 组件（agents/*.jar）

| jar | 组件 | 版本 | 许可 |
|---|---|---|---|
| dbmind-agent-elasticsearch.jar | com.google.code.gson:gson | 2.11.0 | Apache-2.0 |
| dbmind-agent-elasticsearch.jar | com.google.errorprone:error_prone_annotations | 2.27.0 | Apache-2.0 |
| dbmind-agent-jdbc.jar | com.google.code.gson:gson | 2.11.0 | Apache-2.0 |
| dbmind-agent-jdbc.jar | com.google.errorprone:error_prone_annotations | 2.27.0 | Apache-2.0 |
| dbmind-agent-mongodb.jar | com.google.code.gson:gson | 2.11.0 | Apache-2.0 |
| dbmind-agent-mongodb.jar | com.google.errorprone:error_prone_annotations | 2.27.0 | Apache-2.0 |
| dbmind-agent-mongodb.jar | org.mongodb:bson | 5.2.1 | Apache-2.0（由 jar 内类路径推断） |
| dbmind-agent-mongodb.jar | org.mongodb:mongodb-driver-sync | 5.2.1 | Apache-2.0（由 jar 内类路径推断） |
| dbmind-agent-redis.jar | com.google.code.gson:gson | 2.11.0 | Apache-2.0 |
| dbmind-agent-redis.jar | com.google.errorprone:error_prone_annotations | 2.27.0 | Apache-2.0 |
| dbmind-agent-redis.jar | org.apache.commons:commons-pool2 | 2.12.0 | Apache-2.0 |
| dbmind-agent-redis.jar | org.json:json | 20240303 | JSON License（非 OSI，见下） |
| dbmind-agent-redis.jar | org.slf4j:slf4j-api | 1.7.36 | MIT |
| dbmind-agent-redis.jar | redis.clients:jedis | 5.2.0 | MIT |

### org.json（JSON License）说明

`agent-redis.jar` 通过 Jedis 传递引入 `org.json:json`。它使用的不是 OSI 批准的许可，
而是业内俗称的 “JSON License”（正文见下）。该许可**不具传染性、不限制商业使用、
允许再分发**，与 Apache-2.0 组件组合分发不构成冲突；仅在企业开源合规扫描工具中可能
被标记为「非标准许可」，此处附条款原文以便说明。

```
Copyright (c) 2002 JSON.org

Permission is hereby granted, free of charge, to any person obtaining a copy of
this software and associated documentation files (the "Software"), to deal in
the Software without restriction, including without limitation the rights to
use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of
the Software, and to permit persons to whom the Software is furnished to do so,
subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

The Software shall be used for Good, not Evil.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS
FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR
COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER
IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN
CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
```

## 2. 随包分发的原生文件

| 文件 | 来源 | 许可 |
|---|---|---|
| native-auth/mssql-jdbc_auth-12.8.1.x64.dll | mssql-jdbc（Microsoft JDBC Driver for SQL Server） | MIT |

## 3. 内置 Java 运行时（jre/）

由 Eclipse Temurin JDK 21 通过 jlink 裁剪生成，许可为 GPLv2 + Classpath Exception，
可随商业软件自由分发。运行时内每个文件的许可声明保留在 `jre/legal/` 目录。

构建参数（与 CI 一致）：

```
jlink --add-modules java.se,jdk.unsupported,jdk.crypto.ec,jdk.zipfs,jdk.charsets \
      --strip-debug --no-header-files --no-man-pages --compress=zip-6 --output jre
```

## 4. Rust 依赖（编译进 exe）

按许可分组列出；同一组件只出现一次。

### (Apache-2.0 OR MIT) AND BSD-3-Clause（1 个）

encoding_rs 0.8.41

### (MIT OR Apache-2.0) AND Unicode-3.0（1 个）

unicode-ident 1.0.26

### 0BSD OR MIT OR Apache-2.0（1 个）

adler2 2.0.1

### Apache-2.0（3 个）

sync_wrapper 1.0.2、tao 0.37.2、zopfli 0.8.3

### Apache-2.0 / MIT（1 个）

fnv 1.0.7

### Apache-2.0 AND ISC（1 个）

ring 0.17.14

### Apache-2.0 AND MIT（1 个）

dpi 0.1.2

### Apache-2.0 OR BSL-1.0（1 个）

ryu 1.0.23

### Apache-2.0 OR ISC OR MIT（1 个）

rustls 0.23.45

### Apache-2.0 OR MIT（37 个）

atomic-waker 1.1.2、autocfg 1.5.1、bit-set 0.8.0、bit-vec 0.8.0、cargo_toml 1.0.1、const-oid 0.10.2、ctor 1.0.13、equivalent 1.0.2、fastrand 2.5.0、idna_adapter 1.2.2、indexmap 2.14.2、indexmap 1.9.3、libappindicator 0.9.0、libappindicator-sys 0.9.0、muda 0.20.0、multiversion_no_op 1.0.0、ntapi 0.4.3、pin-project-lite 0.2.17、portable-atomic 1.15.0、portable-atomic-util 0.2.8、rustc-hash 2.1.3、tauri 2.12.5、tauri-build 2.7.4、tauri-codegen 2.7.4、tauri-macros 2.7.4、tauri-plugin 2.7.2、tauri-plugin-dialog 2.8.3、tauri-plugin-fs 2.7.1、tauri-runtime 2.12.2、tauri-runtime-wry 2.12.3、tauri-utils 2.10.2、utf8_iter 1.0.4、utf8parse 0.2.2、uuid 1.26.1、window-vibrancy 0.8.1、wry 0.57.1、zeroize 1.9.0

### Apache-2.0 WITH LLVM-exception（1 个）

target-lexicon 0.12.16

### Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT（3 个）

wasi 0.11.1+wasi-snapshot-preview1、wasip2 1.0.4+wasi-0.2.12、wit-bindgen 0.57.1

### Apache-2.0/MIT（3 个）

cesu8 1.1.0、dbus 0.9.12、libdbus-sys 0.2.7

### BSD-2-Clause OR Apache-2.0 OR MIT（2 个）

zerocopy 0.8.57、zerocopy-derive 0.8.57

### BSD-3-Clause（5 个）

alloc-no-stdlib 3.0.0、alloc-no-stdlib 2.0.4、alloc-stdlib 0.3.0、alloc-stdlib 0.2.4、subtle 2.6.1

### BSD-3-Clause AND MIT（2 个）

brotli 9.1.0、brotli 8.0.4

### BSD-3-Clause OR MIT OR Apache-2.0（2 个）

num_enum 0.7.6、num_enum_derive 0.7.6

### BSD-3-Clause/MIT（2 个）

brotli-decompressor 6.2.0、brotli-decompressor 5.0.3

### CC0-1.0 OR MIT-0 OR Apache-2.0（1 个）

dunce 1.0.5

### CDLA-Permissive-2.0（2 个）

webpki-roots 1.0.9、webpki-roots 0.26.11

### ISC（3 个）

libloading 0.7.4、rustls-webpki 0.103.15、untrusted 0.9.0

### MIT（120 个）

atk 0.18.2、atk-sys 0.18.2、axum 0.8.9、axum-core 0.5.6、block2 0.6.2、bytes 1.12.1、cairo-rs 0.18.5、cairo-sys-rs 0.18.2、cargo_metadata 0.19.2、cfb 0.14.0、combine 4.6.8、darling 0.24.1、darling_core 0.24.1、darling_macro 0.24.1、derive_more 2.1.1、derive_more-impl 2.1.1、dlopen2 0.8.2、dlopen2_derive 0.4.3、dom_query 0.28.0、embed-resource 3.0.11、gdk 0.18.2、gdk-pixbuf 0.18.5、gdk-pixbuf-sys 0.18.0、gdk-sys 0.18.2、gdkwayland-sys 0.18.2、gdkx11 0.18.2、gdkx11-sys 0.18.2、generic-array 0.14.7、gio 0.18.4、gio-sys 0.18.1、glib 0.18.5、glib-macros 0.18.5、glib-sys 0.18.1、gobject-sys 0.18.0、gtk 0.18.2、gtk3-macros 0.18.2、gtk-sys 0.18.2、http-body 1.1.0、http-body-util 0.1.5、http-range-header 0.4.2、hyper 1.11.1、hyper-util 0.1.20、ico 0.6.0、infer 0.22.0、javascriptcore-rs 1.1.2、javascriptcore-rs-sys 1.1.1、libredox 0.1.25、libsqlite3-sys 0.30.1、matchers 0.2.0、memoffset 0.9.1、mime_guess 2.0.5、mio 1.2.3、multer 3.1.0、new_debug_unreachable 1.0.6、nu-ansi-term 0.50.3、objc2 0.6.4、objc2-encode 4.1.0、objc2-foundation 0.3.2、openssl-sys 0.9.117、pango 0.18.3、pango-sys 0.18.0、phf 0.13.1、phf_codegen 0.13.1、phf_generator 0.13.1、phf_macros 0.13.1、phf_shared 0.13.1、plist 1.10.1、precomputed-hash 0.1.1、quick-xml 0.42.0、redox_syscall 0.5.18、redox_users 0.5.3、rfd 0.16.1、rusqlite 0.32.1、schemars 1.2.2、schemars 0.9.0、schemars 0.8.22、schemars_derive 0.8.22、sharded-slab 0.1.7、simd-adler32 0.3.10、slab 0.4.12、soup3 0.5.0、soup3-sys 0.5.0、spin 0.9.9、strsim 0.11.1、synstructure 0.14.0、sysinfo 0.32.1、tauri-winres 0.3.6、tokio 1.53.1、tokio-macros 2.7.2、tokio-stream 0.1.19、tokio-util 0.7.19、tower 0.5.3、tower-http 0.6.11、tower-layer 0.3.3、tower-service 0.3.3、tracing 0.1.44、tracing-attributes 0.1.31、tracing-core 0.1.36、tracing-log 0.2.0、tracing-subscriber 0.3.23、try-lock 0.2.5、unsafe-libyaml 0.2.11、urlpattern 0.6.0、valuable 0.1.1、version-compare 0.2.1、vswhom 0.1.0、vswhom-sys 0.1.3、want 0.3.1、webkit2gtk 2.0.2、webkit2gtk-sys 2.0.2、webview2-com 0.39.1、webview2-com-macros 0.8.1、webview2-com-sys 0.39.1、winnow 1.0.4、winnow 0.5.40、winreg 0.55.0、x11 2.21.0、x11-dl 2.21.0、zip 2.4.2、zmij 1.0.23

### MIT AND BSD-3-Clause（1 个）

matchit 0.8.4

### MIT OR Apache-2.0（261 个）

ahash 0.8.12、android_system_properties 0.1.6、anstream 1.0.0、anstyle 1.0.14、anstyle-parse 1.0.0、anstyle-query 1.1.5、anstyle-wincon 3.0.11、anyhow 1.0.104、arbitrary 1.4.2、async-compression 0.4.48、base64 0.23.1、base64 0.22.1、base64 0.21.7、bitflags 2.13.2、block-buffer 0.12.1、block-buffer 0.10.4、bumpalo 3.20.3、camino 1.2.6、cargo-platform 0.1.9、cc 1.4.6、cfg-expr 0.15.8、cfg-if 1.0.5、chrono 0.4.45、clap 4.6.7、clap_builder 4.6.7、clap_derive 4.6.7、clap_lex 1.1.1、colorchoice 1.0.5、compression-codecs 0.4.43、compression-core 0.4.33、cookie 0.18.2、core-foundation 0.10.1、core-foundation-sys 0.8.7、core-graphics 0.25.0、core-graphics-types 0.2.0、cpufeatures 0.2.17、crc32fast 1.5.2、crossbeam-channel 0.5.17、crossbeam-deque 0.8.8、crossbeam-epoch 0.9.21、crossbeam-utils 0.8.23、crypto-common 0.2.2、crypto-common 0.1.7、defmt 1.1.1、defmt-macros 1.1.1、defmt-parser 1.0.0、deranged 0.5.8、derive_arbitrary 1.4.2、digest 0.11.3、digest 0.10.7、dirs 7.0.0、dirs 6.0.0、dirs-sys 0.5.0、displaydoc 0.2.7、dtoa 1.0.11、dyn-clone 1.0.20、either 1.18.0、embed_plist 1.2.2、erased-serde 0.4.10、errno 0.3.14、fdeflate 0.3.7、field-offset 0.3.6、find-msvc-tools 0.1.12、flate2 1.1.10、form_urlencoded 1.2.2、futures-channel 0.3.34、futures-core 0.3.34、futures-executor 0.3.34、futures-io 0.3.34、futures-macro 0.3.34、futures-sink 0.3.34、futures-task 0.3.34、futures-util 0.3.34、getrandom 0.4.3、getrandom 0.3.4、getrandom 0.2.17、glob 0.3.4、hashbrown 0.17.1、hashbrown 0.14.5、hashbrown 0.12.3、hashlink 0.9.1、heck 0.5.0、heck 0.4.1、hex 0.4.3、html5ever 0.39.0、http 1.5.0、httparse 1.10.1、httpdate 1.0.3、hybrid-array 0.4.15、iana-time-zone 0.1.65、iana-time-zone-haiku 0.1.2、idna 1.1.0、ipnet 2.12.2、is_terminal_polyfill 1.70.2、itoa 1.0.18、jni-sys 0.4.1、jni-sys 0.3.1、jni-sys-macros 0.4.1、jsonptr 0.7.1、js-sys 0.3.105、keyboard-types 0.8.3、lazy_static 1.5.0、libc 0.2.189、libssh2-sys 0.3.3、libz-sys 1.1.29、lock_api 0.4.14、log 0.4.34、markup5ever 0.39.0、md-5 0.11.0、mime 0.3.17、multiversion 0.9.0、multiversion-macros 0.9.0、ndk 0.9.0、ndk-context 0.1.1、ndk-sys 0.6.0+11769913、num-conv 0.2.2、num-traits 0.2.19、once_cell 1.21.4、once_cell_polyfill 1.70.2、parking_lot 0.12.5、parking_lot_core 0.9.12、percent-encoding 2.3.2、pkg-config 0.3.34、png 0.18.1、powerfmt 0.2.0、proc-macro2 1.0.107、proc-macro-crate 1.3.1、proc-macro-crate 3.5.0、proc-macro-crate 2.0.2、proc-macro-error 1.0.4、proc-macro-error-attr 1.0.4、quote 1.0.47、rayon 1.12.0、rayon-core 1.13.0、ref-cast 1.0.27、ref-cast-impl 1.0.27、regex 1.13.1、regex-automata 0.4.18、regex-syntax 0.8.11、reqwest 0.13.5、rustc_version 0.4.1、rustls-pki-types 1.15.1、rustversion 1.0.23、scopeguard 1.2.0、semver 1.0.28、serde 1.0.229、serde_core 1.0.229、serde_derive 1.0.229、serde_derive_internals 0.29.1、serde_json 1.0.151、serde_path_to_error 0.1.20、serde_repr 0.1.21、serde_spanned 1.1.1、serde_spanned 0.6.9、serde_with 3.23.0、serde_with_macros 3.23.0、serde_yaml 0.9.34+deprecated、serde-untagged 0.1.9、serialize-to-javascript 0.1.2、serialize-to-javascript-impl 0.1.2、servo_arc 0.4.3、sha2 0.10.9、shlex 2.0.1、signal-hook-registry 1.4.8、simdutf8 0.1.5、smallvec 1.16.1、socket2 0.6.5、socket2 0.5.10、softbuffer 0.4.8、ssh2 0.9.6、stable_deref_trait 1.2.1、string_cache 0.9.0、string_cache_codegen 0.6.1、swift-rs 1.0.9、syn 3.0.6、syn 2.0.119、syn 1.0.109、system-deps 6.2.2、tao-macros 0.1.4、tendril 0.5.1、thiserror 2.0.20、thiserror 1.0.69、thiserror-impl 2.0.20、thiserror-impl 1.0.69、thread_local 1.1.10、time 0.3.55、time-core 0.1.9、time-macros 0.2.32、toml 1.1.6+spec-1.1.0、toml 0.8.2、toml_datetime 1.1.1+spec-1.1.0、toml_datetime 0.6.3、toml_edit 0.25.15+spec-1.1.0、toml_edit 0.20.2、toml_edit 0.19.15、toml_parser 1.1.3+spec-1.1.0、toml_writer 1.1.2+spec-1.1.0、tray-icon 0.25.1、typeid 1.0.3、typenum 1.20.1、unicase 2.9.0、unicode-segmentation 1.13.3、ureq 2.12.1、url 2.5.8、wasm-bindgen 0.2.128、wasm-bindgen-futures 0.4.78、wasm-bindgen-macro 0.2.128、wasm-bindgen-macro-support 0.2.128、wasm-bindgen-shared 0.2.128、wasm-streams 0.5.0、web_atoms 0.2.6、web-sys 0.3.105、web-time 1.1.0、windows 0.57.0、windows 0.62.2、windows_aarch64_gnullvm 0.53.1、windows_aarch64_gnullvm 0.52.6、windows_aarch64_gnullvm 0.42.2、windows_aarch64_msvc 0.53.1、windows_aarch64_msvc 0.42.2、windows_aarch64_msvc 0.52.6、windows_i686_gnu 0.53.1、windows_i686_gnu 0.42.2、windows_i686_gnu 0.52.6、windows_i686_gnullvm 0.53.1、windows_i686_gnullvm 0.52.6、windows_i686_msvc 0.42.2、windows_i686_msvc 0.53.1、windows_i686_msvc 0.52.6、windows_x86_64_gnu 0.53.1、windows_x86_64_gnu 0.52.6、windows_x86_64_gnu 0.42.2、windows_x86_64_gnullvm 0.53.1、windows_x86_64_gnullvm 0.52.6、windows_x86_64_gnullvm 0.42.2、windows_x86_64_msvc 0.53.1、windows_x86_64_msvc 0.42.2、windows_x86_64_msvc 0.52.6、windows-collections 0.3.2、windows-core 0.62.2、windows-core 0.57.0、windows-future 0.3.2、windows-implement 0.60.2、windows-implement 0.57.0、windows-interface 0.59.3、windows-interface 0.57.0、windows-link 0.2.1、windows-numerics 0.3.1、windows-result 0.4.1、windows-result 0.1.2、windows-strings 0.5.1、windows-sys 0.60.2、windows-sys 0.61.2、windows-sys 0.59.0、windows-sys 0.45.0、windows-sys 0.52.0、windows-targets 0.53.5、windows-targets 0.52.6、windows-targets 0.42.2、windows-threading 0.2.1、windows-version 0.1.7

### MIT OR Apache-2.0 OR LGPL-2.1-or-later（2 个）

r-efi 6.0.0、r-efi 5.3.0

### MIT OR Apache-2.0 OR Zlib（2 个）

raw-window-handle 0.6.2、static_vcruntime 3.0.0

### MIT OR Zlib OR Apache-2.0（2 个）

miniz_oxide 0.9.1、miniz_oxide 0.8.9

### MIT/Apache-2.0（19 个）

bitflags 1.3.2、bs58 0.5.1、core_detect 1.0.0、fallible-iterator 0.3.0、fallible-streaming-iterator 0.1.9、foreign-types 0.5.0、foreign-types-macros 0.2.4、foreign-types-shared 0.3.1、ident_case 1.0.1、jni 0.21.1、json-patch 4.2.0、openssl-src 300.6.1+3.6.3、serde_urlencoded 0.7.1、siphasher 1.0.3、vcpkg 0.2.15、version_check 0.9.5、winapi 0.3.9、winapi-i686-pc-windows-gnu 0.4.0、winapi-x86_64-pc-windows-gnu 0.4.0

### MPL-2.0（5 个）

cssparser 0.37.0、cssparser-macros 0.7.1、dtoa-short 0.3.5、option-ext 0.2.0、selectors 0.38.0

### Unicode-3.0（18 个）

icu_collections 2.3.0、icu_locale_core 2.3.0、icu_normalizer 2.3.0、icu_normalizer_data 2.3.0、icu_properties 2.3.0、icu_properties_data 2.3.0、icu_provider 2.3.1、litemap 0.8.3、potential_utf 0.1.6、tinystr 0.8.4、writeable 0.6.4、yoke 0.8.3、yoke-derive 0.8.3、zerofrom 0.1.8、zerofrom-derive 0.1.8、zerotrie 0.2.5、zerovec 0.11.8、zerovec-derive 0.11.6

### Unlicense OR MIT（9 个）

aho-corasick 1.1.5、byteorder 1.5.0、jiff 0.2.37、jiff-core 0.1.1、jiff-static 0.2.37、jiff-tzdb 0.1.8、jiff-tzdb-platform 0.1.3、memchr 2.8.3、winapi-util 0.1.11

### Unlicense/MIT（2 个）

same-file 1.0.6、walkdir 2.5.0

### Zlib（2 个）

foldhash 0.2.0、zlib-rs 0.6.8

### Zlib OR Apache-2.0 OR MIT（17 个）

bytemuck 1.25.2、dispatch2 0.3.1、objc2-app-kit 0.3.2、objc2-cloud-kit 0.3.2、objc2-core-data 0.3.2、objc2-core-foundation 0.3.2、objc2-core-graphics 0.3.2、objc2-core-image 0.3.2、objc2-core-location 0.3.2、objc2-core-text 0.3.2、objc2-exception-helper 0.1.1、objc2-io-surface 0.3.2、objc2-quartz-core 0.3.2、objc2-ui-kit 0.3.2、objc2-user-notifications 0.3.2、objc2-web-kit 0.3.2、tinyvec 1.13.3

## 5. 前端依赖（编译进 web/）

包含构建期依赖（不进产物）与运行时依赖，按许可分组。

### (MPL-2.0 OR Apache-2.0)（1 个）

dompurify 3.4.8

### 0BSD（1 个）

tslib 2.3.0

### Apache-2.0（2 个）

detect-libc 2.1.2、echarts 5.6.0

### Apache-2.0 OR MIT（1 个）

@tauri-apps/api 2.11.1

### BSD-2-Clause（1 个）

entities 7.0.1

### BSD-3-Clause（4 个）

normalize-wheel-es 1.2.0、zrender 5.6.1、source-map-js 1.2.1、moo 0.5.3

### CC0-1.0（1 个）

railroad-diagrams 1.0.0

### ISC（1 个）

picocolors 1.1.1

### MIT（127 个）

state-local 1.0.7、scule 1.3.0、element-plus 2.14.5、monaco-editor 0.56.0、unplugin 3.3.0、tinyglobby 0.2.17、call-bind-apply-helpers 1.0.2、chokidar 5.0.0、mime-types 2.1.35、@types/web-bluetooth 0.0.21、ret 0.1.15、debug 4.4.3、nostics 1.2.0、es-object-atoms 1.1.2、@jridgewell/gen-mapping 0.3.13、ufo 1.6.4、magic-string 0.30.21、@floating-ui/utils 0.2.12、@vue/runtime-core 3.5.42、randexp 0.4.6、perfect-debounce 2.1.0、es-errors 1.3.0、confbox 0.2.4、agent-base 6.0.2、@vue/server-renderer 3.5.42、dunder-proto 1.0.1、@babel/helper-string-parser 7.29.7、memoize-one 6.0.0、@vue/devtools-shared 8.2.1、@rolldown/binding-win32-x64-msvc 1.2.8、quansync 0.2.11、@element-plus/icons-vue 2.3.2、get-intrinsic 1.3.0、@jridgewell/resolve-uri 3.1.2、@rolldown/pluginutils 1.0.1、form-data 4.0.6、axios 1.20.0、@types/dagre 0.7.54、@monaco-editor/loader 1.7.0、@vue/reactivity 3.5.42、@ctrl/tinycolor 4.2.0、@guolao/vue-monaco-editor 1.6.0、magic-string-ast 1.0.3、has-tostringtag 1.0.2、hasown 2.0.4、math-intrinsics 1.1.0、@jridgewell/trace-mapping 0.3.31、dayjs 1.11.23、ast-kit 2.2.0、@vitejs/plugin-vue 6.0.8、@vue-macros/common 3.1.4、fdir 6.5.0、readdirp 5.1.1、@vueuse/core 14.4.0、function-bind 1.1.2、local-pkg 1.2.1、gopd 1.2.0、@vue/compiler-dom 3.5.42、csstype 3.2.3、postcss 8.5.28、@jridgewell/sourcemap-codec 1.6.0、mlly 1.8.2、lodash 4.18.1、@sxzz/popperjs-es 2.11.8、vue-demi 0.14.10、pathe 2.0.3、@jridgewell/remapping 2.3.5、@babel/parser 7.29.8、mime-db 1.52.0、@oxc-project/types 0.149.0、estree-walker 2.0.2、commander 2.20.3、vue 3.5.42、@vue/shared 3.5.42、@babel/types 7.29.8、acorn 8.18.0、delayed-stream 1.0.0、nanoid 3.3.18、get-proto 1.0.1、graphlib 2.1.8、confbox 0.3.1、discontinuous-range 1.0.0、@vue/compiler-core 3.5.42、async-validator 4.2.5、vue-component-type-helpers 3.3.11、proxy-from-env 2.1.0、confbox 0.1.8、exsolve 1.1.1、@vueuse/shared 14.4.0、@vue/compiler-ssr 3.5.42、lodash-unified 1.0.3、es-set-tostringtag 2.1.0、ast-walker-scope 0.9.0、rolldown 1.2.8、vite 8.3.0、birpc 2.9.0、muggle-string 0.4.1、pkg-types 2.3.3、marked 14.0.0、combined-stream 1.0.8、webpack-virtual-modules 0.6.2、@vue/devtools-kit 8.2.1、ms 2.1.3、@vue/devtools-api 8.2.1、@vue/compiler-sfc 3.5.42、follow-redirects 1.16.0、dagre 0.8.5、@types/lodash 4.17.25、@floating-ui/core 1.8.0、@vue/runtime-dom 3.5.42、es-define-property 1.0.1、lodash-es 4.18.1、hookable 5.5.3、vue-router 5.3.1、https-proxy-agent 5.0.1、asynckit 0.4.0、@babel/helper-validator-identifier 7.29.7、@types/lodash-es 4.17.12、@floating-ui/dom 1.8.0、picomatch 4.0.7、sql-formatter 15.8.2、unplugin-utils 0.3.2、@types/trusted-types 2.0.7、@vueuse/metadata 14.4.0、has-symbols 1.1.0、pkg-types 1.3.1、nearley 2.20.1

### MPL-2.0（2 个）

lightningcss 1.33.0、lightningcss-win32-x64-msvc 1.33.0

### Python-2.0（1 个）

argparse 2.0.1

## 6. 不随本产品分发的组件

| 组件 | 说明 |
|---|---|
| 各数据库 JDBC 驱动 | 运行时由用户在本机从厂商渠道下载，存放于用户目录的 .dbmind/drivers 下，适用厂商自身许可（例如 MySQL Connector/J 的 GPLv2 + FOSS Exception、Oracle ojdbc 的 UPL、IBM Db2 的 IBM 许可） |
| Microsoft Edge WebView2 运行时 | 使用系统已安装的运行时（Windows 10/11 自带或由微软分发），不随包分发 |

---

若发现清单与实际分发内容不一致，以 jar / 安装目录中的实际组件为准，并请反馈以便重新生成。
