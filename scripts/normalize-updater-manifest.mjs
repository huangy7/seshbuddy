import { execFileSync } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

/**
 * 规范化更新清单中的下载链接与发布说明。
 *
 * 业务背景：
 * 打包流水线默认生成的更新清单中，分发下载地址容易退化为 API 内部资产端点
 * （例如 api.github.com/repos/.../releases/assets/<id>）。
 * 客户端发起更新下载时无法携带用户身份凭证，会受到严格的匿名调用频次限制（60 次/小时），
 * 容易导致触发 403 Forbidden 阻断更新流程。
 * 同时，客户端更新对话框呈现的更新日志来自清单的 notes 字段，需同步 GitHub Release 的真实正文。
 *
 * 本函数将清单中指向内部 API 的链接归一化为公开可达的 CDN 静态直链（browser_download_url），
 * 处理草稿阶段的临时 untagged 路径固化，并可选同步版本发布日志说明。
 *
 * @param {object} manifest - 解析后的 latest.json 清单对象
 * @param {Array<object>} assets - 发布版本包含的资产列表（来自 release 查询元数据）
 * @param {string} [tagName] - 当前发布的版本标签名称（例如 v0.1.1）
 * @param {string} [releaseNotes] - 当前 Release 的正文发布日志说明
 * @returns {{ manifest: object, mutated: boolean }}
 */
export function normalizeManifest(manifest, assets, tagName = '', releaseNotes = '') {
  if (!manifest || typeof manifest !== 'object' || !manifest.platforms) {
    throw new Error('更新清单格式不合法：缺少 platforms 根字段');
  }

  if (!Array.isArray(assets)) {
    throw new Error('资产列表格式不合法：必须为数组');
  }

  const assetByApiUrl = new Map();
  const assetById = new Map();

  for (const asset of assets) {
    if (!asset || typeof asset !== 'object') continue;

    if (asset.apiUrl) {
      assetByApiUrl.set(asset.apiUrl, asset);
    }

    if (asset.id !== undefined && asset.id !== null) {
      assetById.set(String(asset.id), asset);
    }

    const apiUrl = asset.apiUrl || '';
    const numericMatch = apiUrl.match(/releases\/assets\/(\d+)/);
    if (numericMatch) {
      assetById.set(numericMatch[1], asset);
    }
  }

  let mutated = false;
  const nextPlatforms = {};

  for (const [platformKey, platformData] of Object.entries(manifest.platforms)) {
    if (!platformData || typeof platformData.url !== 'string') {
      nextPlatforms[platformKey] = platformData;
      continue;
    }

    const currentUrl = platformData.url;
    let targetAsset = null;

    if (assetByApiUrl.has(currentUrl)) {
      targetAsset = assetByApiUrl.get(currentUrl);
    } else {
      const match = currentUrl.match(/releases\/assets\/(\d+)/);
      if (match && assetById.has(match[1])) {
        targetAsset = assetById.get(match[1]);
      }
    }

    let resolvedUrl = currentUrl;
    if (targetAsset && targetAsset.url) {
      resolvedUrl = targetAsset.url;
    }

    // 草稿发布阶段的下载链接包含临时 untagged 占位符，
    // 正式发布后该路径失效，需按确定 tag 固化为稳定下载路径
    if (tagName && resolvedUrl.includes('/download/untagged-')) {
      resolvedUrl = resolvedUrl.replace(
        /\/download\/(untagged-[^/]+)\//,
        `/download/${encodeURIComponent(tagName)}/`,
      );
    }

    if (resolvedUrl !== currentUrl) {
      nextPlatforms[platformKey] = {
        ...platformData,
        url: resolvedUrl,
      };
      mutated = true;
    } else {
      nextPlatforms[platformKey] = platformData;
    }
  }

  let nextNotes = manifest.notes;
  if (typeof releaseNotes === 'string' && releaseNotes.trim()) {
    const trimmedNotes = releaseNotes.trim();
    if (trimmedNotes !== manifest.notes) {
      nextNotes = trimmedNotes;
      mutated = true;
    }
  }

  return {
    manifest: {
      ...manifest,
      notes: nextNotes,
      platforms: nextPlatforms,
    },
    mutated,
  };
}

/**
 * 运行命令行入口：下载最新清单、执行归一化修复并覆写上传
 */
export function runCli(argv = process.argv, env = process.env) {
  const tagName = argv[2] || env.GITHUB_REF_NAME;
  if (!tagName) {
    console.error('[FAIL] 必须提供 release tag 名称（例如 node scripts/normalize-updater-manifest.mjs v0.1.1）');
    process.exit(1);
  }

  console.log(`[INFO] 开始检查 release ${tagName} 的更新清单...`);

  const viewOutput = execFileSync('gh', ['release', 'view', tagName, '--json', 'assets,tagName,body'], {
    encoding: 'utf-8',
    env,
  });
  const releaseInfo = JSON.parse(viewOutput);
  const assets = releaseInfo.assets || [];
  const releaseBody = typeof releaseInfo.body === 'string' ? releaseInfo.body.trim() : '';

  const tempDir = mkdtempSync(join(tmpdir(), 'seshbuddy-manifest-'));
  const tempManifestPath = join(tempDir, 'latest.json');

  try {
    try {
      execFileSync('gh', ['release', 'download', tagName, '-p', 'latest.json', '-O', tempManifestPath], {
        encoding: 'utf-8',
        env,
      });
    } catch (err) {
      console.error(`[FAIL] 无法下载 ${tagName} 的 latest.json：`, err.message);
      process.exit(1);
    }

    const rawJson = readFileSync(tempManifestPath, 'utf-8');
    const manifest = JSON.parse(rawJson);

    // 如果 GitHub Release 正文不是打包器默认占位符，则同步至更新清单的 notes
    const effectiveNotes =
      releaseBody && releaseBody !== 'See the assets to download this version and install.'
        ? releaseBody
        : '';

    const { manifest: updatedManifest, mutated } = normalizeManifest(manifest, assets, tagName, effectiveNotes);

    if (!mutated) {
      console.log('[INFO] 所有平台的下载链接均已是公开直链且更新日志一致，无需变更。');
      return;
    }

    writeFileSync(tempManifestPath, JSON.stringify(updatedManifest, null, 2) + '\n', 'utf-8');

    console.log('[INFO] 检测到更新清单需要同步，正在重新上传 latest.json...');
    execFileSync('gh', ['release', 'upload', '--clobber', tagName, tempManifestPath], {
      encoding: 'utf-8',
      env,
    });
    console.log('[INFO] latest.json 更新成功！');
  } finally {
    try {
      rmSync(tempDir, { recursive: true, force: true });
    } catch {
      // 临时目录清理失败不影响主流程
    }
  }
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  runCli();
}
