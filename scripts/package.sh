#!/usr/bin/env bash
set -euo pipefail

PROFILE="${1:-release}"
TARGET="${2:-}"

# 判断操作系统环境
IS_WINDOWS=false
case "$(uname -s)" in
    MINGW*|MSYS*|CYGWIN*) IS_WINDOWS=true ;;
esac
if [ "${RUNNER_OS:-}" = "Windows" ]; then
    IS_WINDOWS=true
fi
if [[ "${TARGET}" == *"windows"* ]]; then
    IS_WINDOWS=true
fi

# 确定包标识目标架构
if [ -z "${TARGET}" ]; then
    PACKAGE_TARGET=$(rustc -vV 2>/dev/null | sed -n 's/^host: //p' || true)
    if [ -z "${PACKAGE_TARGET}" ]; then
        PACKAGE_TARGET="$(uname -m)-unknown-linux-gnu"
    fi
else
    PACKAGE_TARGET="${TARGET}"
fi

# 确定版本号
if [[ "${GITHUB_REF_NAME:-}" =~ ^v[0-9] ]]; then
    VERSION="${GITHUB_REF_NAME#v}"
else
    VERSION=$(cargo metadata --no-deps --format-version 1 2>/dev/null | grep -o '"version":"[^"]*"' | head -n 1 | cut -d'"' -f4 || true)
    if [ -z "${VERSION}" ]; then
        VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)
    fi
    if [ -z "${VERSION}" ]; then
        VERSION="${GITHUB_SHA::7}"
    fi
    if [ -z "${VERSION}" ]; then
        VERSION="dev"
    fi
fi

BIN_NAME="ags"
EXT=""
if [ "${IS_WINDOWS}" = "true" ]; then
    EXT=".exe"
fi

# 定位编译生成的二进制文件
SRC_BIN=""
if [ -n "${TARGET}" ] && [ -f "target/${TARGET}/${PROFILE}/${BIN_NAME}${EXT}" ]; then
    SRC_BIN="target/${TARGET}/${PROFILE}/${BIN_NAME}${EXT}"
elif [ -f "target/${PROFILE}/${BIN_NAME}${EXT}" ]; then
    SRC_BIN="target/${PROFILE}/${BIN_NAME}${EXT}"
elif [ -n "${PACKAGE_TARGET}" ] && [ -f "target/${PACKAGE_TARGET}/${PROFILE}/${BIN_NAME}${EXT}" ]; then
    SRC_BIN="target/${PACKAGE_TARGET}/${PROFILE}/${BIN_NAME}${EXT}"
else
    echo "Error: Binary not found for profile '${PROFILE}' and target '${PACKAGE_TARGET}'." >&2
    echo "Checked candidates:" >&2
    if [ -n "${TARGET}" ]; then echo "  - target/${TARGET}/${PROFILE}/${BIN_NAME}${EXT}" >&2; fi
    echo "  - target/${PROFILE}/${BIN_NAME}${EXT}" >&2
    if [ -n "${PACKAGE_TARGET}" ]; then echo "  - target/${PACKAGE_TARGET}/${PROFILE}/${BIN_NAME}${EXT}" >&2; fi
    exit 1
fi

ARCHIVE_DIR="${BIN_NAME}-${VERSION}-${PACKAGE_TARGET}"
mkdir -p "${ARCHIVE_DIR}"
mkdir -p dist

echo "Packaging ${SRC_BIN} -> dist/ (target: ${PACKAGE_TARGET}, profile: ${PROFILE}, version: ${VERSION})..."

if [ "${IS_WINDOWS}" = "true" ]; then
    ARCHIVE_FILE="${ARCHIVE_DIR}.zip"

    cp "${SRC_BIN}" "${ARCHIVE_DIR}/"
    cp README.md LICENSE CHANGELOG.md "${ARCHIVE_DIR}/" 2>/dev/null || true

    if command -v 7z >/dev/null 2>&1; then
        7z a "dist/${ARCHIVE_FILE}" "${ARCHIVE_DIR}"
    elif command -v zip >/dev/null 2>&1; then
        zip -r "dist/${ARCHIVE_FILE}" "${ARCHIVE_DIR}"
    elif command -v powershell >/dev/null 2>&1; then
        powershell -Command "Compress-Archive -Path '${ARCHIVE_DIR}' -DestinationPath 'dist/${ARCHIVE_FILE}' -Force"
    else
        echo "Error: neither 7z, zip, nor powershell found for creating zip archive" >&2
        rm -rf "${ARCHIVE_DIR}"
        exit 1
    fi

    if command -v sha256sum >/dev/null 2>&1; then
        (cd dist && sha256sum "${ARCHIVE_FILE}" > "${ARCHIVE_FILE}.sha256")
    elif command -v shasum >/dev/null 2>&1; then
        (cd dist && shasum -a 256 "${ARCHIVE_FILE}" > "${ARCHIVE_FILE}.sha256")
    else
        certutil -hashfile "dist/${ARCHIVE_FILE}" SHA256 | findstr -v "hash" | findstr -v "CertUtil" > "dist/${ARCHIVE_FILE}.sha256"
    fi
else
    ARCHIVE_FILE="${ARCHIVE_DIR}.tar.gz"

    cp "${SRC_BIN}" "${ARCHIVE_DIR}/"
    cp README.md LICENSE CHANGELOG.md "${ARCHIVE_DIR}/" 2>/dev/null || true

    tar -czf "dist/${ARCHIVE_FILE}" "${ARCHIVE_DIR}"

    if command -v sha256sum >/dev/null 2>&1; then
        (cd dist && sha256sum "${ARCHIVE_FILE}" > "${ARCHIVE_FILE}.sha256")
    elif command -v shasum >/dev/null 2>&1; then
        (cd dist && shasum -a 256 "${ARCHIVE_FILE}" > "${ARCHIVE_FILE}.sha256")
    else
        echo "Error: neither sha256sum nor shasum found" >&2
        rm -rf "${ARCHIVE_DIR}"
        exit 1
    fi
fi

# 清理打包中间临时目录
rm -rf "${ARCHIVE_DIR}"

echo "Packaging completed successfully:"
echo "  - dist/${ARCHIVE_FILE}"
echo "  - dist/${ARCHIVE_FILE}.sha256"
