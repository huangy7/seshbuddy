#!/usr/bin/env node
/**
 * 校验「更新签名私钥」与「tauri.conf.json 里 pin 的公钥」是否配对。
 *
 * 为什么需要这一步：构建脚本只能检查 `TAURI_SIGNING_PRIVATE_KEY` 是否**非空**，
 * 却检查不出它**是不是对的那一把**。而密钥配错不会报任何错 —— 它会安静地产出一个
 * 签名无效的更新包，直到用户更新失败才会发现。本脚本把这种错误提前到构建开始前。
 *
 * 用法：
 *   node scripts/verify-updater-key.mjs <被签名的文件> <签名文件> <tauri.conf.json>
 *
 * 退出码：0 = 配对；1 = 不配对或参数/格式有误。
 *
 * 实现说明：Tauri 的签名是 minisign 格式的 Ed25519。
 * - 签名文件本身是 base64，解码后才是 minisign 文本
 * - 公钥字段同样是 base64，解码后第二行是 `sig_alg[2] + key_id[8] + pubkey[32]`
 * - `ED` 表示对文件的 BLAKE2b-512 摘要签名，`Ed` 表示对文件原文签名
 */

import { createHash, createPublicKey, verify } from "node:crypto";
import { readFileSync } from "node:fs";

function fail(msg) {
  console.error(`[verify-updater-key] ${msg}`);
  process.exit(1);
}

const [dataPath, sigPath, confPath] = process.argv.slice(2);
if (!dataPath || !sigPath || !confPath) {
  fail("用法: verify-updater-key.mjs <dataFile> <sigFile> <tauri.conf.json>");
}

/** minisign 文本的第二行是 base64 载荷；第一行是 untrusted comment。 */
function payloadOf(minisignText, what) {
  const lines = minisignText.trim().split("\n");
  if (lines.length < 2) fail(`${what} 格式异常：缺少载荷行`);
  return Buffer.from(lines[1], "base64");
}

let sigBlob;
try {
  // 签名文件整体是 base64，先解一层
  sigBlob = payloadOf(Buffer.from(readFileSync(sigPath, "utf8").trim(), "base64").toString(), "签名文件");
} catch (e) {
  fail(`读取签名失败: ${e.message}`);
}

let pubBlob;
try {
  const cfg = JSON.parse(readFileSync(confPath, "utf8"));
  const pubkey = cfg?.plugins?.updater?.pubkey;
  if (typeof pubkey !== "string" || !pubkey.trim()) {
    fail(`${confPath} 里没有 plugins.updater.pubkey`);
  }
  pubBlob = payloadOf(Buffer.from(pubkey, "base64").toString(), "公钥");
} catch (e) {
  fail(`读取公钥失败: ${e.message}`);
}

const sigAlg = sigBlob.subarray(0, 2).toString();
const sigKeyId = sigBlob.subarray(2, 10).reverse().toString("hex").toUpperCase();
const signature = sigBlob.subarray(10, 74);

const pubKeyId = pubBlob.subarray(2, 10).reverse().toString("hex").toUpperCase();
const publicKey = pubBlob.subarray(10, 42);

if (sigAlg !== "ED" && sigAlg !== "Ed") fail(`未知的签名算法: ${sigAlg}`);
if (sigKeyId !== pubKeyId) {
  fail(`签名用的 key_id (${sigKeyId}) 与配置里的公钥 key_id (${pubKeyId}) 不一致`);
}

const data = readFileSync(dataPath);
const digest = sigAlg === "ED" ? createHash("blake2b512").update(data).digest() : data;

// Ed25519 裸公钥需要包一层 SPKI DER 头才能交给 Node 的 crypto
const SPKI_ED25519_PREFIX = Buffer.from("302a300506032b6570032100", "hex");
const key = createPublicKey({
  key: Buffer.concat([SPKI_ED25519_PREFIX, publicKey]),
  format: "der",
  type: "spki",
});

if (!verify(null, digest, key, signature)) {
  fail("验签未通过：私钥与配置里的公钥不配对");
}

console.log(`[verify-updater-key] ✓ 私钥与配置公钥配对（key_id ${pubKeyId}）`);
