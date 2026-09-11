#!/usr/bin/env bash

#============================================================
# File: relaydrop.sh
# Description: 文件传输中继服务
# Source: https://github.com/jetsung/relaydrop
# URL: https://fx4.cn/relaydrop
# Author: Jetsung Chan <i@jetsung.com>
# Version: 0.1.0
# CreatedAt: 2026-07-14
# UpdatedAt: 2026-07-14
#============================================================

if [[ -n "${DEBUG:-}" ]]; then
    set -eux
else
    set -euo pipefail
fi

CDN_URL="${CDN:-https://fastfile.asfd.cn/}"

USER_ID="$(id -u)"

check_is_command() {
    command -v "$1" >/dev/null 2>&1
}

check_in_china() {
    if [[ -n "${CN:-}" ]]; then
        return 0 # 手动指定
    fi
    if [[ "$(curl -s -m 3 -o /dev/null -w "%{http_code}" https://www.google.com)" == "000" ]]; then
        return 0 # 中国网络
    fi
    return 1 # 非中国网络
}

# 若为 https://xxx.xx 不以 / 结尾，则组合时去掉加速网址的 https://
#   格式为 https://file.xxx.io/github.com/
# 若为 https://xxx.xx/ 以 / 结尾，则组合时保留加速网址的 https://
#   格式为 https://xxx.xx/https://github.com/
check_remove_https() {
    if [[ -n "$1" && "${1: -1}" != "/" ]]; then
        echo 1
    fi
}

do_remove_https() {
    local url="$1"
    if [[ -n "$NO_HTTPS" ]]; then
        # shellcheck disable=SC2001
        echo "$url" | sed 's|https:/||2'

    else
        echo "$url"
    fi
}

########################## 以上为通用函数 #########################

# relaydrop 资产命名规则（goreleaser）：
#   relaydrop-v0.1.1-{arch}-{vendor}-{os}.tar.xz
#   arch:    x86_64 | aarch64 | loongarch64
#   vendor:  unknown | apple | pc
#   os:      linux-gnu | linux-musl | apple-darwin | windows-msvc
get_download_url() {
    repo_api_url=$(do_remove_https "${CDN_URL}https://api.github.com/repos/${1}/releases/latest")
    curl -fsSL "$repo_api_url" | jq -r --arg arch "$ARCH" --arg vendor "$VENDOR" --arg os "$OS" '.assets[] | select(.name | test("\($arch)-\($vendor)-\($os)")) | select(.name | test("\\.(tar\\.xz|tar\\.gz|zip)$")) | .browser_download_url'
}

# Windows 资产为 zip，其余为 tar.xz / tar.gz
get_archive_ext() {
    if [[ "$OS" == "windows-msvc" ]]; then
        echo "zip"
    else
        echo "tar.xz"
    fi
}

download_exact() {
    local download_file
    download_file="tmp.$(get_archive_ext)"
    local file_bin="relaydrop"
    TMP_DIR=$(mktemp -d /tmp/relaydrop.XXXXXX)

    cleanup() {
        rm -rf -- "$TMP_DIR"
    }
    trap cleanup EXIT

    pushd "$TMP_DIR" >/dev/null

    # 若非 root 用户，则不使用 sudo，安装到用户目录
    local _sudo=""
    if [[ "$USER_ID" -eq 0 ]]; then
        _bin_path="/usr/local/bin"
    else
        _sudo=""
        _bin_path="$HOME/.local/bin"
    fi

    if [[ -z "${CUSTOM_URL:-}" ]]; then
        _download_url=$(do_remove_https "${CDN_URL}${DOWNLOAD_URL}")
    else
        _download_url="$CUSTOM_URL"
    fi

    if ! curl -fsSL "$_download_url" -o "$download_file"; then
        echo "Error: Failed to download $download_file"
        exit 1
    fi

    if [[ "$download_file" == *.zip ]]; then
        if ! unzip -oq "$download_file"; then
            echo "Error: Extraction failed"
            rm -f "$download_file"
            exit 1
        fi
    else
        if ! tar -xf "$download_file"; then
            echo "Error: Extraction failed"
            rm -f "$download_file"
            exit 1
        fi
    fi

    $_sudo mkdir -p "$_bin_path"
    $_sudo mv "$file_bin" "$_bin_path/"

    popd >/dev/null
}

main() {
    # 解析命令行参数
    CUSTOM_URL=""
    while [[ $# -gt 0 ]]; do
        case "$1" in
            --url)
                CUSTOM_URL="$2"
                shift 2
                ;;
            *)
                echo "Unknown option: $1"
                exit 1
                ;;
        esac
    done

    # 优先级：命令行参数 > 环境变量 > 默认流程
    # 将 URL 环境变量也赋值给 CUSTOM_URL，确保 download_exact() 中逻辑正确
    if [[ -z "${CUSTOM_URL:-}" && -n "${URL:-}" ]]; then
        CUSTOM_URL="$URL"
    fi
    DOWNLOAD_URL="${CUSTOM_URL:-${URL:-}}"

    OS="$(uname | tr '[:upper:]' '[:lower:]')"
    ARCH="$(uname -m | tr '[:upper:]' '[:lower:]')"

    case "$OS" in
        linux)
            VENDOR="unknown"
            case "$ARCH" in
                x86_64)  ARCH="x86_64";  OS="linux-gnu" ;;
                aarch64|arm64) ARCH="aarch64"; OS="linux-gnu" ;;
                loongarch64) ARCH="loongarch64"; OS="linux-gnu" ;;
                *) echo "Unsupported architecture: $ARCH"; exit 1 ;;
            esac
            ;;
        darwin)
            VENDOR="apple"
            case "$ARCH" in
                x86_64)  ARCH="x86_64";  OS="darwin" ;;
                aarch64|arm64) ARCH="aarch64"; OS="darwin" ;;
                *) echo "Unsupported architecture: $ARCH"; exit 1 ;;
            esac
            ;;
        *)
            echo "Unsupported OS: $OS"
            exit 1
            ;;
    esac

    if [[ -z "$DOWNLOAD_URL" ]]; then
        if ! check_in_china; then
            CDN_URL=""
        fi

        NO_HTTPS=$(check_remove_https "$CDN_URL")

        DOWNLOAD_URL="$(get_download_url jetsung/relaydrop)"

        if [[ -z "$DOWNLOAD_URL" || "$DOWNLOAD_URL" == "null" ]]; then
            echo "Error: Could not find a download URL for $ARCH-$VENDOR-$OS"
            exit 1
        fi
    else
        echo "使用指定下载地址: $DOWNLOAD_URL"
    fi

    download_exact

    echo ""

    if ! check_is_command "relaydrop"; then
        echo "relaydrop has not been installed successfully."
        echo ""
        exit 1
    fi

    echo ""
    echo "relaydrop has been installed successfully!"
    echo ""
    relaydrop --help
    echo ""
    relaydrop --version
    echo ""
}

main "$@"
