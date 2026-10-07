import { describe, expect, it } from 'vitest';
import { normalizeManifest } from './normalize-updater-manifest.mjs';

describe('normalize-updater-manifest 脚本测试', () => {
  it('正确将 API 资产链接替换为公开直链', () => {
    const sampleManifest = {
      version: '0.1.1',
      notes: '版本说明',
      pub_date: '2026-10-06T15:08:42.951Z',
      platforms: {
        'darwin-aarch64': {
          signature: 'sig-arm',
          url: 'https://api.github.com/repos/huangy7/seshbuddy/releases/assets/615851706',
        },
        'windows-x86_64': {
          signature: 'sig-win',
          url: 'https://api.github.com/repos/huangy7/seshbuddy/releases/assets/615847524',
        },
        'linux-x86_64': {
          signature: 'sig-linux',
          url: 'https://api.github.com/repos/huangy7/seshbuddy/releases/assets/615899999',
        },
      },
    };

    const assets = [
      {
        id: 615851706,
        apiUrl: 'https://api.github.com/repos/huangy7/seshbuddy/releases/assets/615851706',
        url: 'https://github.com/huangy7/seshbuddy/releases/download/v0.1.1/SeshBuddy_aarch64.app.tar.gz',
        name: 'SeshBuddy_aarch64.app.tar.gz',
      },
      {
        id: 615847524,
        apiUrl: 'https://api.github.com/repos/huangy7/seshbuddy/releases/assets/615847524',
        url: 'https://github.com/huangy7/seshbuddy/releases/download/v0.1.1/SeshBuddy_x64.msi',
        name: 'SeshBuddy_x64.msi',
      },
      {
        id: 615899999,
        apiUrl: 'https://api.github.com/repos/huangy7/seshbuddy/releases/assets/615899999',
        url: 'https://github.com/huangy7/seshbuddy/releases/download/v0.1.1/SeshBuddy_amd64.AppImage.tar.gz',
        name: 'SeshBuddy_amd64.AppImage.tar.gz',
      },
    ];

    const result = normalizeManifest(sampleManifest, assets, 'v0.1.1');
    expect(result.mutated).toBe(true);
    expect(result.manifest.platforms['darwin-aarch64'].url).toBe(
      'https://github.com/huangy7/seshbuddy/releases/download/v0.1.1/SeshBuddy_aarch64.app.tar.gz',
    );
    expect(result.manifest.platforms['windows-x86_64'].url).toBe(
      'https://github.com/huangy7/seshbuddy/releases/download/v0.1.1/SeshBuddy_x64.msi',
    );
    expect(result.manifest.platforms['linux-x86_64'].url).toBe(
      'https://github.com/huangy7/seshbuddy/releases/download/v0.1.1/SeshBuddy_amd64.AppImage.tar.gz',
    );
  });

  it('如果链接已是公开直链，则不做修改并标记未变更', () => {
    const sampleManifest = {
      version: '0.1.1',
      platforms: {
        'darwin-aarch64': {
          signature: 'sig-arm',
          url: 'https://github.com/huangy7/seshbuddy/releases/download/v0.1.1/SeshBuddy_aarch64.app.tar.gz',
        },
      },
    };

    const assets = [
      {
        id: 615851706,
        apiUrl: 'https://api.github.com/repos/huangy7/seshbuddy/releases/assets/615851706',
        url: 'https://github.com/huangy7/seshbuddy/releases/download/v0.1.1/SeshBuddy_aarch64.app.tar.gz',
        name: 'SeshBuddy_aarch64.app.tar.gz',
      },
    ];

    const result = normalizeManifest(sampleManifest, assets, 'v0.1.1');
    expect(result.mutated).toBe(false);
    expect(result.manifest.platforms['darwin-aarch64'].url).toBe(
      'https://github.com/huangy7/seshbuddy/releases/download/v0.1.1/SeshBuddy_aarch64.app.tar.gz',
    );
  });

  it('将草稿阶段的临时 untagged 路径按发布 tag 固化', () => {
    const sampleManifest = {
      version: '0.1.1',
      platforms: {
        'darwin-aarch64': {
          signature: 'sig-arm',
          url: 'https://api.github.com/repos/huangy7/seshbuddy/releases/assets/615851706',
        },
      },
    };

    const assets = [
      {
        id: 615851706,
        apiUrl: 'https://api.github.com/repos/huangy7/seshbuddy/releases/assets/615851706',
        url: 'https://github.com/huangy7/seshbuddy/releases/download/untagged-abc123/SeshBuddy_aarch64.app.tar.gz',
        name: 'SeshBuddy_aarch64.app.tar.gz',
      },
    ];

    const result = normalizeManifest(sampleManifest, assets, 'v0.1.1');
    expect(result.mutated).toBe(true);
    expect(result.manifest.platforms['darwin-aarch64'].url).toBe(
      'https://github.com/huangy7/seshbuddy/releases/download/v0.1.1/SeshBuddy_aarch64.app.tar.gz',
    );
  });

  it('同步更新 Release 正文到更新清单的 notes 字段', () => {
    const sampleManifest = {
      version: '0.1.1',
      notes: 'See the assets to download this version and install.',
      platforms: {
        'darwin-aarch64': {
          signature: 'sig-arm',
          url: 'https://github.com/huangy7/seshbuddy/releases/download/v0.1.1/SeshBuddy_aarch64.app.tar.gz',
        },
      },
    };

    const assets = [
      {
        id: 615851706,
        apiUrl: 'https://api.github.com/repos/huangy7/seshbuddy/releases/assets/615851706',
        url: 'https://github.com/huangy7/seshbuddy/releases/download/v0.1.1/SeshBuddy_aarch64.app.tar.gz',
        name: 'SeshBuddy_aarch64.app.tar.gz',
      },
    ];

    const result = normalizeManifest(sampleManifest, assets, 'v0.1.1', '- 支持 Linux 平台\n- 修复更新链接');
    expect(result.mutated).toBe(true);
    expect(result.manifest.notes).toBe('- 支持 Linux 平台\n- 修复更新链接');
  });

  it('如果 Release 正文未变或为空则不修改 notes', () => {
    const sampleManifest = {
      version: '0.1.1',
      notes: '现有发布说明',
      platforms: {
        'darwin-aarch64': {
          signature: 'sig-arm',
          url: 'https://github.com/huangy7/seshbuddy/releases/download/v0.1.1/SeshBuddy_aarch64.app.tar.gz',
        },
      },
    };

    const assets = [
      {
        id: 615851706,
        apiUrl: 'https://api.github.com/repos/huangy7/seshbuddy/releases/assets/615851706',
        url: 'https://github.com/huangy7/seshbuddy/releases/download/v0.1.1/SeshBuddy_aarch64.app.tar.gz',
        name: 'SeshBuddy_aarch64.app.tar.gz',
      },
    ];

    const result = normalizeManifest(sampleManifest, assets, 'v0.1.1', '现有发布说明');
    expect(result.mutated).toBe(false);
    expect(result.manifest.notes).toBe('现有发布说明');
  });

  it('输入非法格式时抛出明确异常', () => {
    expect(() => normalizeManifest(null, [])).toThrow('缺少 platforms 根字段');
    expect(() => normalizeManifest({ platforms: {} }, null)).toThrow('必须为数组');
  });
});
