#!/bin/sh
# 在本机钥匙串里创建一个自签名的代码签名证书（只需运行一次）。
#
# 为什么需要：未签名 / 临时签名的 app，系统按每次编译的哈希识别，
# 每次重新打包后「屏幕录制」「辅助功能」等权限都会失效。
# 用固定证书签名后，系统按「证书 + 包名」识别，重新打包权限依然有效。
#
# 证书只保存在本机登录钥匙串，名称为「wtools Local Signing」，有效期 10 年；
# 不需要信任设置、不需要管理员密码。删除方法：钥匙串访问 → 登录 → 我的证书 → 删除该证书。
set -eu

NAME="wtools Local Signing"
KEYCHAIN="$HOME/Library/Keychains/login.keychain-db"

if security find-certificate -c "$NAME" "$KEYCHAIN" >/dev/null 2>&1; then
  echo "证书「$NAME」已存在，无需重复创建。"
  exit 0
fi

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

cat > "$TMP/openssl.cnf" <<CFG
[req]
distinguished_name = dn
prompt = no
[dn]
CN = $NAME
[ext]
basicConstraints = critical,CA:false
keyUsage = critical,digitalSignature
extendedKeyUsage = critical,codeSigning
CFG

# 系统自带的 openssl（LibreSSL）生成的 p12 能被 security 直接导入
/usr/bin/openssl req -x509 -newkey rsa:2048 -nodes -days 3650 \
  -keyout "$TMP/key.pem" -out "$TMP/cert.pem" \
  -config "$TMP/openssl.cnf" -extensions ext 2>/dev/null
/usr/bin/openssl pkcs12 -export -inkey "$TMP/key.pem" -in "$TMP/cert.pem" \
  -out "$TMP/cert.p12" -passout pass:wtools -name "$NAME"

# -T 允许 codesign 使用这把私钥
security import "$TMP/cert.p12" -k "$KEYCHAIN" -P wtools -T /usr/bin/codesign >/dev/null

echo "已创建证书「$NAME」。之后运行 pnpm release 会自动用它签名。"
echo "首次签名时如果弹出「codesign 想要使用钥匙串中的密钥」，请输入登录密码并点「始终允许」。"
