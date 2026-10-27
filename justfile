set shell := ["bash", "-uc"]

# 默认列出可用 recipe
default:
    @just --list

# 构建二进制文件 (debug / release，可选 target)
build profile="release" target="":
    @flags="" && \
    if [ "{{profile}}" = "release" ]; then flags="--release"; \
    elif [ "{{profile}}" != "debug" ] && [ -n "{{profile}}" ]; then flags="--profile {{profile}}"; fi && \
    if [ -n "{{target}}" ]; then flags="${flags} --target {{target}}"; fi && \
    if [ "${USE_CROSS:-false}" = "true" ]; then \
        cross build ${flags} --bin ags; \
    else \
        cargo build ${flags} --bin ags; \
    fi

# 打包构建产物至 dist/ 目录 (生成归档与 sha256 校验和)
package profile="release" target="": (build profile target)
    @bash scripts/package.sh "{{profile}}" "{{target}}"

# 快速检查代码语法与类型
check:
    cargo check --all-targets --all-features

# 检查指定架构的代码语法与类型，并验证二进制构建
check-target target:
    cargo check --target {{target}} --all-targets --all-features
    cargo build --target {{target}} --bin ags

# 运行全量单元测试
test *args:
    cargo test --all-targets --all-features {{args}}

# 格式化代码 (支持透传参数，例如: just fmt -- --check)
fmt *args:
    @if command -v cargo-fmt >/dev/null 2>&1 || cargo --list | grep -q "^    fmt"; then \
        cargo fmt --all {{args}}; \
    else \
        echo "cargo-fmt not installed, skipping fmt"; \
    fi

# 代码静态分析与规范检查
clippy:
    @if command -v cargo-clippy >/dev/null 2>&1 || cargo --list | grep -q "^    clippy"; then \
        cargo clippy --all-targets --all-features -- -D warnings; \
    else \
        cargo check --all-targets --all-features; \
    fi

# 安装 ags 二进制至 ~/.cargo/bin
install:
    cargo install --path .

# 本地运行 ags
run *args:
    cargo run --bin ags -- {{args}}

# 生成或更新 CHANGELOG.md (依赖 git-cliff)
changelog:
    @if command -v git-cliff >/dev/null 2>&1; then \
        git-cliff --output CHANGELOG.md; \
        echo "CHANGELOG.md updated successfully via git-cliff"; \
    else \
        echo "git-cliff is not installed. Install with 'cargo install git-cliff' to generate changelog."; \
    fi

# 版本升级辅助 (例如: just bump 0.2.0)
bump version:
    @sed -i 's/^version = ".*"/version = "{{version}}"/' Cargo.toml
    @cargo check
    @echo "Bumped agent-switch version to {{version}}"
