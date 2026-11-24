// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 MOSSLS

// 启动时自动加载外部效果 helper（mos_灯光混合、mos_消散、mos_扫光、mos_打光 等）
(function () {
    try {
        app.executeCommand("effect.helpers.scan");
    } catch (e) {
        // 权限关闭或扫描失败时静默，用户仍可用 效果 ▸ 扫描效果辅助程序...
    }
})();
