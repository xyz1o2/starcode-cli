#!/usr/bin/env bash
# 只在一次性 GitHub 托管 runner 上替换默认钥匙串，不接触开发者登录库。
set -euo pipefail
if [[ "$(uname -s)" != Darwin || "${RUNNER_ENVIRONMENT:-}" != github-hosted || "${GITHUB_ACTIONS:-}" != true ]]; then
  printf '%s\n' 'This test requires a disposable GitHub-hosted macOS runner.' >&2
  exit 2
fi

scratch="$(mktemp -d "${RUNNER_TEMP:?}/starcode-keychain.XXXXXX")"
keychain="$scratch/regression.keychain-db"
# 公开的测试密码，仅用于本次 runner 中的临时库。
password='starcode-public-keychain-test-password'
original_default="$(security default-keychain -d user | tr -d '"' | xargs)"
original_search=()
while IFS= read -r entry; do
  entry="${entry#*\"}"
  entry="${entry%\"*}"
  [[ -z "$entry" ]] || original_search+=("$entry")
done < <(security list-keychains -d user)

cleanup() {
  # 恢复 runner 原配置后删除本脚本唯一创建的测试库。
  security default-keychain -d user -s "$original_default" || true
  security list-keychains -d user -s "${original_search[@]}" || true
  security delete-keychain "$keychain" || true
  rm -rf "$scratch"
}
trap cleanup EXIT
security create-keychain -p "$password" "$keychain"
security set-keychain-settings -lut 600 "$keychain"
security unlock-keychain -p "$password" "$keychain"
security default-keychain -d user -s "$keychain"
security list-keychains -d user -s "$keychain"
export STARCODE_TEST_KEYCHAIN="$keychain"
cargo test --locked --lib core::secure_storage::keychain::tests::native_store_updates_and_remains_readable_by_existing_commands -- --ignored --exact --test-threads=1
