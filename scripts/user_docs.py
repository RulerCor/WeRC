"""Stage product manuals; keep internal audits in the workspace."""
from pathlib import Path
import argparse
import shutil
from release_version import VERSION, RELEASE_NAME

ROOT = Path(__file__).resolve().parent.parent
README = f'''WeRC v{VERSION}
Windows 10 1903+ / Windows 11 x64

运行 werc-desktop.exe，跟随首次全屏引导连接微信；需要 Bot 时授权并复制连接配置。
应用密码默认关闭，可在设置中自行开启。
请保留完整程序目录。缺少 WebView2 时，运行随包 WebView2RuntimeInstaller.exe。
请自行下载并安装微信；原生收发支持 4.2.0.53（推荐）、4.2.0.52、4.1.15.13 和 4.1.13.7 的匹配 x64 构建，具体能力以连接检测结果为准。

使用说明：usage.md
API 文档：API接口文档.html（离线可看）、bot-client.md、openapi.json
兼容与限制：release-notes.md
仅非商业用途；分享时请保留 licenses 和 open-source-components。
'''
PACKAGE_README = f'''WeRC v{VERSION}

解压完整目录后，双击「启动WeRC.cmd」；首次跟随全屏引导连接微信。
资料保存在本启动脚本旁的 ReviewData 文件夹中，更新前请备份并保留。
应用密码默认关闭，需要时可在设置中开启。
请自行下载并安装微信。原生收发支持 4.2.0.53（推荐）、4.2.0.52、4.1.15.13 和 4.1.13.7 的匹配 x64 构建。
缺少 WebView2 时，运行 {RELEASE_NAME}/WebView2RuntimeInstaller.exe。

API 接入：双击「API接口文档.html」离线查看。
使用与兼容说明：{RELEASE_NAME} 目录中的 usage.md 和 release-notes.md。
退出 WeRC 后替换完整程序目录；不要删除自己的资料目录。
仅非商业用途；分享时保留随包许可证和开源组件资料。
'''
INTERNAL_REPORTS = ('服务器验收报告.md', '功能复查与API文档.md', '剩余验收边界.md',
                    '首次设置与目标核对.md', '微信与分发补充.md')


def stage_user_docs(destination):
    destination = Path(destination)
    destination.mkdir(parents=True, exist_ok=True)
    for name in ('usage.md', 'release-notes.md'):
        shutil.copy2(ROOT / 'docs' / name, destination / name)
    (destination / 'README.txt').write_text(README, encoding='utf-8')


def stage_package_docs(package):
    package = Path(package)
    stage_user_docs(package / RELEASE_NAME)
    (package / 'README.txt').write_text(PACKAGE_README, encoding='utf-8')
    for name in (*INTERNAL_REPORTS, '启动验收版.cmd'):
        (package / name).unlink(missing_ok=True)


def verify_user_docs(destination):
    destination = Path(destination)
    assert (destination / 'README.txt').read_text(encoding='utf-8') == README
    for name in ('usage.md', 'release-notes.md'):
        assert (destination / name).read_bytes() == (ROOT / 'docs' / name).read_bytes(), name


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('destination', type=Path)
    parser.add_argument('--package', action='store_true')
    args = parser.parse_args()
    if args.package:
        stage_package_docs(args.destination)
    else:
        stage_user_docs(args.destination)
