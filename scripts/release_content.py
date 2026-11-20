"""Release content policy for user-provided WeChat installations."""
from pathlib import Path


def verify_no_wechat_installer(directory):
    root = Path(directory)
    for path in root.rglob('*'):
        if not path.is_file():
            continue
        parts = [part.lower() for part in path.relative_to(root).parts]
        name = path.name.lower()
        if ('vendor-installers' in parts and 'wechat' in parts) or (
            name.endswith(('.exe', '.msi', '.msix')) and
            (name.startswith(('weixin_', 'wechat_', 'weixinsetup', 'wechatsetup')))
        ):
            raise ValueError('WeChat installer must not be included: ' + str(path.relative_to(root)))
