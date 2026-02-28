#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Agent Ruler installer

Usage:
  bash install/install.sh --local
  bash install/install.sh --release [--version vX.Y.Z]
  bash install/install.sh --uninstall [--purge-installs] [--purge-data]

Notes:
  --local           Build from local source and install a dev binary.
  --release         Download + verify a GitHub Release binary and install it.
  --version         Release tag for --release (default: latest release).
  --uninstall       Remove ~/.local/bin/agent-ruler (symlink or file).
  --purge-installs  Also remove ~/.local/share/agent-ruler/installs/*.
  --purge-data      Also remove Agent Ruler runtime data under XDG data projects dir.
EOF
}

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
DEFAULT_RELEASE_ARCHIVE="agent-ruler-linux-x86_64.tar.gz"
DEFAULT_RELEASE_SUMS="SHA256SUMS.txt"

require_cmd() {
  local cmd="$1"
  if ! command -v "${cmd}" >/dev/null 2>&1; then
    echo "[install] required command not found: ${cmd}" >&2
    exit 1
  fi
}

stop_managed_runtime_processes() {
  local maybe_binary="$1"
  if [[ "${AGENT_RULER_INSTALL_SKIP_STOP:-0}" == "1" ]]; then
    echo "[install] skipping managed runtime stop (AGENT_RULER_INSTALL_SKIP_STOP=1)"
    return 0
  fi
  if [[ -x "${maybe_binary}" ]]; then
    echo "[install] stopping managed runtime processes before replacing binary"
    "${maybe_binary}" run -- openclaw gateway stop >/dev/null 2>&1 || true
    "${maybe_binary}" ui stop >/dev/null 2>&1 || true
  fi
}

stop_managed_runtime_processes_for_reinstall() {
  local preferred_binary="$1"
  local link_path="${HOME}/.local/bin/agent-ruler"
  local linked_binary=""
  local command_binary=""

  if [[ -x "${preferred_binary}" ]]; then
    stop_managed_runtime_processes "${preferred_binary}"
    return 0
  fi

  if [[ -L "${link_path}" ]]; then
    linked_binary="$(readlink -f "${link_path}" || true)"
    if [[ -x "${linked_binary}" ]]; then
      stop_managed_runtime_processes "${linked_binary}"
      return 0
    fi
  fi

  command_binary="$(command -v agent-ruler 2>/dev/null || true)"
  if [[ -x "${command_binary}" ]]; then
    stop_managed_runtime_processes "${command_binary}"
  fi
}

detect_github_repo() {
  # explicit override (forks/private)
  if [[ -n "${AGENT_RULER_GITHUB_REPO:-}" ]]; then
    printf '%s' "${AGENT_RULER_GITHUB_REPO}"
    return 0
  fi

  # default for normal users (no repo checkout)
  printf '%s' "steadeepanda/agent-ruler"
  return 0
}

curl_auth_headers() {
  if [[ -n "${GITHUB_TOKEN:-}" ]]; then
    printf '%s\n' "-H" "Authorization: Bearer ${GITHUB_TOKEN}"
  fi
}

github_api_get() {
  local url="$1"
  local extra_headers=()
  while IFS= read -r part; do
    [[ -n "${part}" ]] && extra_headers+=("${part}")
  done < <(curl_auth_headers)

  curl -fsSL -H "Accept: application/vnd.github+json" "${extra_headers[@]}" "${url}"
}

github_api_download_asset_by_id() {
  local repo="$1"
  local asset_id="$2"
  local out="$3"
  local extra_headers=()
  while IFS= read -r part; do
    [[ -n "${part}" ]] && extra_headers+=("${part}")
  done < <(curl_auth_headers)

  curl -fsSL --retry 3 --retry-delay 1 -L \
    -H "Accept: application/octet-stream" \
    "${extra_headers[@]}" \
    -o "${out}" \
    "https://api.github.com/repos/${repo}/releases/assets/${asset_id}"
}

github_download_file() {
  local url="$1"
  local out="$2"
  local extra_headers=()
  while IFS= read -r part; do
    [[ -n "${part}" ]] && extra_headers+=("${part}")
  done < <(curl_auth_headers)

  curl -fsSL --retry 3 --retry-delay 1 -L "${extra_headers[@]}" -o "${out}" "${url}"
}

json_first_match() {
  local json="$1"
  local regex="$2"
  printf '%s' "${json}" | grep -oE "${regex}" | head -n1 | sed -E 's/^[^"]*"[^"]*"[[:space:]]*:[[:space:]]*"([^"]*)".*/\1/'
}

json_release_asset_id_by_name() {
  local json="$1"
  local name="$2"
  local compact_json
  compact_json="$(printf '%s' "${json}" | tr -d '[:space:]')"
  # Match asset objects specifically by their API asset URL path to avoid
  # confusing the release-level "id" with asset ids.
  printf '%s' "${compact_json}" | awk -v want="${name}" '
    {
      line=$0
      n=split(line, parts, "\"url\":\"https://api.github.com/repos/")
      for (i=2; i<=n; i++) {
        segment="\"url\":\"https://api.github.com/repos/" parts[i]
        if (segment !~ /\/releases\/assets\/[0-9]+/) {
          continue
        }
        if (index(segment, "\"name\":\"" want "\"") == 0) {
          continue
        }
        if (match(segment, /\/releases\/assets\/[0-9]+/)) {
          id_path=substr(segment, RSTART, RLENGTH)
          gsub(/[^0-9]/, "", id_path)
          if (id_path != "") {
            print id_path
            exit
          }
        }
      }
    }
  '
}

resolve_release_metadata() {
  local repo="$1"
  local version="${2:-}"
  local endpoint
  if [[ -n "${version}" ]]; then
    endpoint="https://api.github.com/repos/${repo}/releases/tags/${version}"
  else
    endpoint="https://api.github.com/repos/${repo}/releases/latest"
  fi

  local json
  if ! json="$(github_api_get "${endpoint}")"; then
    echo "[install] failed to resolve GitHub release metadata from ${endpoint}" >&2
    exit 1
  fi

  local tag
  tag="$(json_first_match "${json}" '"tag_name"[[:space:]]*:[[:space:]]*"[^"]+"')"
  if [[ -z "${tag}" ]]; then
    echo "[install] release metadata did not include tag_name" >&2
    exit 1
  fi

  local archive_url sums_url archive_id sums_id
  archive_url="$(json_first_match "${json}" '"browser_download_url"[[:space:]]*:[[:space:]]*"[^"]*agent-ruler-linux-x86_64\.tar\.gz"')"
  sums_url="$(json_first_match "${json}" '"browser_download_url"[[:space:]]*:[[:space:]]*"[^"]*SHA256SUMS\.txt"')"
  archive_id="$(json_release_asset_id_by_name "${json}" "${DEFAULT_RELEASE_ARCHIVE}" || true)"
  sums_id="$(json_release_asset_id_by_name "${json}" "${DEFAULT_RELEASE_SUMS}" || true)"

  if [[ -z "${archive_url}" && -z "${archive_id}" ]]; then
    echo "[install] release assets missing ${DEFAULT_RELEASE_ARCHIVE}" >&2
    exit 1
  fi
  if [[ -z "${sums_url}" && -z "${sums_id}" ]]; then
    echo "[install] release assets missing ${DEFAULT_RELEASE_SUMS}" >&2
    exit 1
  fi
  # Output:
  # 0 tag
  # 1 archive_url
  # 2 sums_url
  # 3 archive_id
  # 4 sums_id
  printf '%s\n%s\n%s\n%s\n%s\n' "${tag}" "${archive_url}" "${sums_url}" "${archive_id}" "${sums_id}"
}

link_into_path_dir_if_possible() {
  local install_binary="$1"
  local fallback_link_dir="$2"
  local selected_dir=""

  local preferred_dirs=("${HOME}/.local/bin" "${HOME}/.cargo/bin" "${HOME}/bin")
  local pref
  for pref in "${preferred_dirs[@]}"; do
    case ":${PATH:-}:" in
      *":${pref}:"*)
        if [[ -d "${pref}" && -w "${pref}" ]]; then
          selected_dir="${pref}"
          break
        fi
        ;;
    esac
  done

  local path_entry
  if [[ -z "${selected_dir}" ]]; then
    IFS=':' read -ra path_parts <<< "${PATH:-}"
    for path_entry in "${path_parts[@]}"; do
      [[ -z "${path_entry}" || "${path_entry}" == "." ]] && continue
      if [[ -d "${path_entry}" && -w "${path_entry}" ]]; then
        selected_dir="${path_entry}"
        break
      fi
    done
  fi

  if [[ -z "${selected_dir}" ]]; then
    selected_dir="${fallback_link_dir}"
  fi

  mkdir -p "${selected_dir}"
  local selected_link="${selected_dir}/agent-ruler"
  ln -sfn "${install_binary}" "${selected_link}"
  echo "${selected_link}"
}

ensure_path_persistence_if_needed() {
  local link_dir="$1"
  if command -v agent-ruler >/dev/null 2>&1; then
    return 0
  fi

  local export_line='export PATH="$HOME/.local/bin:$PATH"'
  local updated=0
  local profile
  for profile in "${HOME}/.bashrc" "${HOME}/.zshrc" "${HOME}/.profile"; do
    if [[ ! -f "${profile}" ]]; then
      touch "${profile}"
    fi
    if ! grep -Fq "${export_line}" "${profile}"; then
      printf '\n%s\n' "${export_line}" >> "${profile}"
      updated=1
    fi
  done

  if [[ "${updated}" == "1" ]]; then
    echo "[install] added PATH bootstrap to ~/.bashrc, ~/.zshrc, and ~/.profile"
  fi
  echo "[install] if this shell still cannot find agent-ruler, open a new terminal session"
}

align_current_shell_resolution() {
  local install_binary="$1"
  local resolved_path=""
  resolved_path="$(command -v agent-ruler 2>/dev/null || true)"
  if [[ -z "${resolved_path}" ]]; then
    return 0
  fi
  if [[ "${resolved_path}" != "${HOME}/"* ]]; then
    return 0
  fi
  if [[ ! -w "$(dirname "${resolved_path}")" ]]; then
    return 0
  fi

  ln -sfn "${install_binary}" "${resolved_path}"
  echo "[install] refreshed existing PATH-preferred link ${resolved_path}"
}

prune_stale_install_instances() {
  local install_parent="$1"
  local keep_name="$2"

  if [[ ! -d "${install_parent}" ]]; then
    return 0
  fi

  local entry
  for entry in "${install_parent}"/*; do
    [[ -d "${entry}" ]] || continue
    local name
    name="$(basename "${entry}")"
    if [[ "${name}" == "${keep_name}" || "${name}" == "bridge" || "${name}" == "docs-site" ]]; then
      continue
    fi
    echo "[install] removing stale install instance ${entry}"
    rm -rf "${entry}"
  done
}

install_local() {
  local build_target="${REPO_ROOT}/target/release/agent-ruler"
  local install_root="${HOME}/.local/share/agent-ruler/installs/dev"
  local install_parent="${HOME}/.local/share/agent-ruler/installs"
  local install_bridge_root="${install_parent}/bridge"
  local install_binary="${install_root}/agent-ruler"
  local link_dir="${HOME}/.local/bin"
  local link_path="${link_dir}/agent-ruler"
  local staging_binary=""

  echo "[install] building local release binary"
  (
    cd "${REPO_ROOT}"
    cargo build --release
  )

  if [[ ! -x "${build_target}" ]]; then
    echo "[install] build output missing: ${build_target}" >&2
    exit 1
  fi

  stop_managed_runtime_processes_for_reinstall "${install_binary}"

  echo "[install] installing to ${install_binary}"
  mkdir -p "${install_root}"
  staging_binary="$(mktemp "${install_root}/agent-ruler.new.XXXXXX")"
  install -m 755 "${build_target}" "${staging_binary}"
  mv -f "${staging_binary}" "${install_binary}"

  # Keep runner bridge assets in sync with the local source tree so OpenClaw
  # uses the same tool adapter version as the freshly built binary.
  echo "[install] syncing bridge assets to ${install_bridge_root}"
  rm -rf "${install_bridge_root}"
  mkdir -p "${install_bridge_root}"
  cp -a "${REPO_ROOT}/bridge/." "${install_bridge_root}/"

  # Keep only the current local install instance to avoid stale binary reuse.
  prune_stale_install_instances "${install_parent}" "dev"

  echo "[install] updating user symlink ${link_path}"
  mkdir -p "${link_dir}"
  ln -sfn "${install_binary}" "${link_path}"

  local path_link
  path_link="$(link_into_path_dir_if_possible "${install_binary}" "${link_dir}")"
  echo "[install] active command link ${path_link}"

  ensure_path_persistence_if_needed "${link_dir}"
  align_current_shell_resolution "${install_binary}"

  local next_cmd="agent-ruler init"
  if ! command -v agent-ruler >/dev/null 2>&1; then
    next_cmd="${path_link} init"
  fi

  cat <<EOF
[install] done
- binary: ${install_binary}
- symlink: ${link_path}
- command-link: ${path_link}

Next:
  ${next_cmd}
EOF
}

install_release() {
  local requested_version="${1:-}"
  local install_parent="${HOME}/.local/share/agent-ruler/installs"
  local install_bridge_root="${install_parent}/bridge"
  local install_docs_root="${install_parent}/docs-site"
  local link_dir="${HOME}/.local/bin"
  local link_path="${link_dir}/agent-ruler"
  local repo tag archive_url sums_url archive_id sums_id install_name install_root install_binary
  local work_dir archive_path sums_path checksums_filtered extract_dir extracted_binary staging_binary path_link next_cmd

  require_cmd curl
  require_cmd tar
  require_cmd sha256sum

  repo="$(detect_github_repo)"
  if ! mapfile -t release_meta < <(resolve_release_metadata "${repo}" "${requested_version}"); then
    echo "[install] failed to resolve release metadata for ${repo}" >&2
    if [[ -z "${GITHUB_TOKEN:-}" ]]; then
      echo "[install] if this is a private repository, set GITHUB_TOKEN and retry." >&2
    fi
    exit 1
  fi
  if [[ "${#release_meta[@]}" -lt 5 ]]; then
    echo "[install] invalid release metadata returned for ${repo}" >&2
    exit 1
  fi
  tag="${release_meta[0]}"
  archive_url="${release_meta[1]}"
  sums_url="${release_meta[2]}"
  archive_id="${release_meta[3]}"
  sums_id="${release_meta[4]}"

  install_name="${tag}"
  install_root="${install_parent}/${install_name}"
  install_binary="${install_root}/agent-ruler"

  echo "[install] release repo: ${repo}"
  echo "[install] release tag: ${tag}"

  work_dir="$(mktemp -d)"
  archive_path="${work_dir}/${DEFAULT_RELEASE_ARCHIVE}"
  sums_path="${work_dir}/${DEFAULT_RELEASE_SUMS}"
  checksums_filtered="${work_dir}/SHA256SUMS.filtered"
  extract_dir="${work_dir}/extract"
  mkdir -p "${extract_dir}"

  trap 'rm -rf "${work_dir}"' RETURN

  echo "[install] downloading ${DEFAULT_RELEASE_ARCHIVE}"
  if [[ -n "${archive_id}" ]]; then
    github_api_download_asset_by_id "${repo}" "${archive_id}" "${archive_path}"
  elif [[ -n "${archive_url}" ]]; then
    github_download_file "${archive_url}" "${archive_path}"
  else
    echo "[install] unable to resolve download source for ${DEFAULT_RELEASE_ARCHIVE}" >&2
    exit 1
  fi
  echo "[install] downloading ${DEFAULT_RELEASE_SUMS}"
  if [[ -n "${sums_id}" ]]; then
    github_api_download_asset_by_id "${repo}" "${sums_id}" "${sums_path}"
  elif [[ -n "${sums_url}" ]]; then
    github_download_file "${sums_url}" "${sums_path}"
  else
    echo "[install] unable to resolve download source for ${DEFAULT_RELEASE_SUMS}" >&2
    exit 1
  fi

  awk -v archive="${DEFAULT_RELEASE_ARCHIVE}" '
    $NF ~ (archive "$") { print $1 "  " archive; found=1 }
    END { if (found != 1) exit 1 }
  ' "${sums_path}" > "${checksums_filtered}" || {
    echo "[install] checksum entry for ${DEFAULT_RELEASE_ARCHIVE} missing in ${DEFAULT_RELEASE_SUMS}" >&2
    exit 1
  }
  (
    cd "${work_dir}"
    sha256sum -c "$(basename "${checksums_filtered}")"
  )

  tar -xzf "${archive_path}" -C "${extract_dir}"
  extracted_binary="$(find "${extract_dir}" -maxdepth 2 -type f -name agent-ruler | head -n1 || true)"
  if [[ -z "${extracted_binary}" || ! -f "${extracted_binary}" ]]; then
    echo "[install] extracted archive did not contain agent-ruler binary" >&2
    exit 1
  fi

  if [[ -d "${extract_dir}/bridge" ]]; then
    echo "[install] syncing bridge assets to ${install_bridge_root}"
    rm -rf "${install_bridge_root}"
    mkdir -p "${install_bridge_root}"
    cp -a "${extract_dir}/bridge/." "${install_bridge_root}/"
  fi

  if [[ -d "${extract_dir}/docs-site" ]]; then
    echo "[install] syncing docs bundle to ${install_docs_root}"
    rm -rf "${install_docs_root}"
    mkdir -p "${install_docs_root}"
    cp -a "${extract_dir}/docs-site/." "${install_docs_root}/"
  fi

  stop_managed_runtime_processes_for_reinstall "${install_binary}"

  echo "[install] installing to ${install_binary}"
  mkdir -p "${install_root}"
  staging_binary="$(mktemp "${install_root}/agent-ruler.new.XXXXXX")"
  install -m 755 "${extracted_binary}" "${staging_binary}"
  mv -f "${staging_binary}" "${install_binary}"

  prune_stale_install_instances "${install_parent}" "${install_name}"

  echo "[install] updating user symlink ${link_path}"
  mkdir -p "${link_dir}"
  ln -sfn "${install_binary}" "${link_path}"

  path_link="$(link_into_path_dir_if_possible "${install_binary}" "${link_dir}")"
  echo "[install] active command link ${path_link}"

  ensure_path_persistence_if_needed "${link_dir}"
  align_current_shell_resolution "${install_binary}"

  next_cmd="agent-ruler init"
  if ! command -v agent-ruler >/dev/null 2>&1; then
    next_cmd="${path_link} init"
  fi

  cat <<EOF
[install] done
- binary: ${install_binary}
- symlink: ${link_path}
- command-link: ${path_link}

Next:
  ${next_cmd}
EOF
}

uninstall_local() {
  local xdg_data_home="${XDG_DATA_HOME:-${HOME}/.local/share}"
  local data_root="${xdg_data_home}/agent-ruler"
  local installs_root="${data_root}/installs"
  local link_path="${HOME}/.local/bin/agent-ruler"
  local remove_installs="${1:-0}"
  local purge_data="${2:-0}"
  local stop_binary=""

  if [[ -L "${link_path}" ]]; then
    local linked_binary
    linked_binary="$(readlink -f "${link_path}" || true)"
    if [[ -n "${linked_binary}" && -x "${linked_binary}" ]]; then
      stop_binary="${linked_binary}"
    fi
  fi
  if [[ -z "${stop_binary}" ]] && command -v agent-ruler >/dev/null 2>&1; then
    stop_binary="$(command -v agent-ruler)"
  fi
  if [[ -n "${stop_binary}" ]]; then
    echo "[uninstall] attempting managed gateway stop via ${stop_binary} run -- openclaw gateway stop"
    if ! "${stop_binary}" run -- openclaw gateway stop >/dev/null 2>&1; then
      echo "[uninstall] managed gateway stop command failed or not configured; continuing"
    fi
    echo "[uninstall] attempting managed UI stop via ${stop_binary} ui stop"
    if ! "${stop_binary}" ui stop >/dev/null 2>&1; then
      echo "[uninstall] managed UI stop command failed or not running; continuing"
    fi
  fi

  if [[ -L "${link_path}" ]]; then
    local target
    target="$(readlink -f "${link_path}" || true)"
    echo "[uninstall] removing symlink ${link_path}"
    rm -f "${link_path}"
    if [[ -n "${target}" && "${target}" == "${installs_root}"/* && -f "${target}" ]]; then
      echo "[uninstall] removing linked binary ${target}"
      rm -f "${target}"
    fi
  elif [[ -f "${link_path}" ]]; then
    echo "[uninstall] removing file ${link_path}"
    rm -f "${link_path}"
  else
    echo "[uninstall] no ${link_path} found"
  fi

  if [[ "${remove_installs}" == "1" ]]; then
    if [[ -d "${installs_root}" ]]; then
      echo "[uninstall] removing installs under ${installs_root}"
      rm -rf "${installs_root}"
    else
      echo "[uninstall] no installs directory found at ${installs_root}"
    fi
  fi

  if [[ "${purge_data}" == "1" ]]; then
    local projects_root="${data_root}/projects"
    if command -v agent-ruler >/dev/null 2>&1; then
      echo "[uninstall] attempting current-project purge via agent-ruler purge --yes"
      if ! agent-ruler purge --yes >/dev/null 2>&1; then
        echo "[uninstall] current-project purge command failed or not initialized; continuing"
      fi
    fi
    if [[ -d "${projects_root}" ]]; then
      echo "[uninstall] removing runtime data under ${projects_root}"
      rm -rf "${projects_root}"
    else
      echo "[uninstall] no runtime data directory found at ${projects_root}"
    fi
  fi

  echo "[uninstall] done"
}

if [[ $# -lt 1 ]]; then
  usage
  exit 1
fi

action=""
purge_installs=0
purge_data=0
release_version=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --local)
      action="local"
      ;;
    --release)
      action="release"
      ;;
    --version)
      shift
      if [[ $# -eq 0 ]]; then
        echo "--version requires a value (example: --version v0.1.5)" >&2
        usage
        exit 1
      fi
      release_version="$1"
      ;;
    --uninstall)
      action="uninstall"
      ;;
    --purge-installs)
      purge_installs=1
      ;;
    --purge-data)
      purge_data=1
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "unknown option: $1" >&2
      usage
      exit 1
      ;;
  esac
  shift
done

if [[ -z "${action}" ]]; then
  echo "missing action flag (--local, --release, or --uninstall)" >&2
  usage
  exit 1
fi

if [[ "${action}" != "uninstall" && ( "${purge_installs}" == "1" || "${purge_data}" == "1" ) ]]; then
  echo "--purge-installs/--purge-data are only valid with --uninstall" >&2
  usage
  exit 1
fi

if [[ "${action}" != "release" && -n "${release_version}" ]]; then
  echo "--version is only valid with --release" >&2
  usage
  exit 1
fi

if [[ -n "${release_version}" && "${release_version}" != v* ]]; then
  release_version="v${release_version}"
fi

case "${action}" in
  local)
    install_local
    ;;
  release)
    install_release "${release_version}"
    ;;
  uninstall)
    uninstall_local "${purge_installs}" "${purge_data}"
    ;;
  *)
    echo "unsupported action: ${action}" >&2
    usage
    exit 1
    ;;
esac
