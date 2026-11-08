//! The interface's words in English and Chinese. The language follows the system's (the browser's
//! on the web) until the person picks one in the top bar; the pick is remembered.
//!
//! Every word the interface shows is a field of [`Strings`], so a language that misses one does
//! not compile. Templates take their values in order with `{}` ([`fill`]). What the engine and the
//! server say (a refusal's reason) stays as they say it.

/// A language of the interface.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Lang {
    /// English.
    En,
    /// Simplified Chinese.
    Zh,
}

impl Lang {
    /// The system's language, when it is one of ours; English otherwise.
    pub fn detect() -> Lang {
        sys_locale::get_locale().as_deref().and_then(Lang::from_code).unwrap_or(Lang::En)
    }

    /// `zh`, `zh-CN`, `zh_Hans`… is Chinese; `en…` English.
    pub fn from_code(code: &str) -> Option<Lang> {
        let code = code.to_ascii_lowercase();
        if code.starts_with("zh") {
            Some(Lang::Zh)
        } else if code.starts_with("en") {
            Some(Lang::En)
        } else {
            None
        }
    }

    /// The code it is stored as.
    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Zh => "zh",
        }
    }

    /// Its words.
    pub(crate) fn strings(self) -> &'static Strings {
        match self {
            Lang::En => &EN,
            Lang::Zh => &ZH,
        }
    }

    /// A command's name, for the status bar: `clip.split` is "Split" or "分割".
    pub fn command(self, id: &str) -> &'static str {
        let (en, zh) = match id {
            "project.setCanvas" => ("Canvas settings", "画布设置"),
            "media.import" => ("Import", "导入素材"),
            "track.add" => ("Add track", "添加轨道"),
            "track.set" => ("Track settings", "轨道设置"),
            "clip.add" => ("Add clip", "添加片段"),
            "clip.split" => ("Split", "分割"),
            "clip.trim" => ("Trim", "裁剪"),
            "clip.move" => ("Move", "移动"),
            "clip.remove" => ("Delete", "删除"),
            "clip.setSpeed" => ("Speed", "变速"),
            "clip.set" => ("Clip properties", "片段属性"),
            "clip.addEffect" => ("Add effect", "添加效果"),
            "clip.setKey" => ("Add keyframe", "添加关键帧"),
            "clip.removeKey" => ("Remove keyframe", "删除关键帧"),
            "clip.clearKeys" => ("Clear keyframes", "清除关键帧"),
            "clip.setAnimation" => ("Animation", "动画"),
            "edit.undo" => ("Undo", "撤销"),
            "edit.redo" => ("Redo", "重做"),
            "edit.batch" => ("Several edits", "批量编辑"),
            "captions.import" => ("Import captions", "导入字幕"),
            "captions.export" => ("Export captions", "导出字幕"),
            "title.templates" => ("Title templates", "花字模板"),
            "sync" => ("Sync", "同步"),
            _ => return "",
        };
        if self == Lang::Zh { zh } else { en }
    }

    /// A track's name as shown: the engine's default names ("Video 2") read in this language;
    /// names people gave are shown as they are.
    pub fn track_name(self, name: &str) -> String {
        if self == Lang::En {
            return name.to_string();
        }
        if name == reelforge_engine::CAPTIONS_TRACK {
            return "字幕".into();
        }
        let Some((word, n)) = name.split_once(' ') else { return name.to_string() };
        if !n.chars().all(|c| c.is_ascii_digit()) {
            return name.to_string();
        }
        match word {
            "Video" => format!("视频 {n}"),
            "Text" => format!("文字 {n}"),
            "Audio" => format!("音频 {n}"),
            _ => name.to_string(),
        }
    }
}

/// `template` with each `{}` replaced by the next of `values`.
pub fn fill(template: &str, values: &[&dyn std::fmt::Display]) -> String {
    let mut out = String::with_capacity(template.len() + 16);
    let mut rest = template;
    let mut values = values.iter();
    while let Some(i) = rest.find("{}") {
        out.push_str(&rest[..i]);
        if let Some(v) = values.next() {
            out.push_str(&v.to_string());
        }
        rest = &rest[i + 2..];
    }
    out.push_str(rest);
    out
}

/// Every word the interface shows. Fields are named for where the word appears.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))] // the browser has no Export
pub(crate) struct Strings {
    pub(crate) ready: &'static str,
    /// The language switch: the other language's name.
    pub(crate) switch_to: &'static str,
    pub(crate) switch_tip: &'static str,
    pub(crate) joining: &'static str,
    pub(crate) joined: &'static str,
    pub(crate) unreachable: &'static str,
    pub(crate) no_copy: &'static str,
    pub(crate) done: &'static str,
    pub(crate) sending: &'static str,
    pub(crate) imported: &'static str,
    pub(crate) undo_tip: &'static str,
    pub(crate) redo_tip: &'static str,
    pub(crate) connecting: &'static str,
    pub(crate) live: &'static str,
    pub(crate) syncing: &'static str,
    pub(crate) live_tip: &'static str,
    pub(crate) to_start: &'static str,
    pub(crate) prev_frame: &'static str,
    pub(crate) play_tip: &'static str,
    pub(crate) next_frame: &'static str,
    pub(crate) to_end: &'static str,
    pub(crate) frame: &'static str,
    pub(crate) split: &'static str,
    pub(crate) split_tip: &'static str,
    pub(crate) delete: &'static str,
    pub(crate) delete_tip: &'static str,
    pub(crate) title: &'static str,
    pub(crate) title_tip: &'static str,
    pub(crate) new_title: &'static str,
    pub(crate) colour: &'static str,
    pub(crate) colour_tip: &'static str,
    pub(crate) drop_hint: &'static str,
    pub(crate) drop_overlay: &'static str,
    pub(crate) fit_tip: &'static str,
    pub(crate) zoom_in_tip: &'static str,
    pub(crate) zoom_out_tip: &'static str,
    pub(crate) snap_on: &'static str,
    pub(crate) snap_off: &'static str,
    pub(crate) gpu_preview: &'static str,
    pub(crate) cpu_preview: &'static str,
    pub(crate) preview_failed: &'static str,
    pub(crate) no_sound: &'static str,
    pub(crate) select_hint: &'static str,
    pub(crate) shortcuts: &'static str,
    pub(crate) keys: [(&'static str, &'static str); 8],
    pub(crate) kind_video: &'static str,
    pub(crate) kind_picture: &'static str,
    pub(crate) kind_sound: &'static str,
    pub(crate) kind_title: &'static str,
    pub(crate) kind_colour: &'static str,
    pub(crate) sec_text: &'static str,
    pub(crate) sec_templates: &'static str,
    pub(crate) captions_imported: &'static str,
    pub(crate) sec_colour: &'static str,
    pub(crate) sec_transform: &'static str,
    pub(crate) sec_sound: &'static str,
    pub(crate) sec_motion: &'static str,
    pub(crate) anim_in: &'static str,
    pub(crate) anim_out: &'static str,
    pub(crate) anim_none: &'static str,
    pub(crate) anim_length: &'static str,
    /// In [`reelforge_model::AnimKind::ALL`] order.
    pub(crate) anim_kinds: [&'static str; 9],
    /// The same presets as they leave.
    pub(crate) anim_out_kinds: [&'static str; 9],
    pub(crate) key_add: &'static str,
    pub(crate) key_remove: &'static str,
    pub(crate) size: &'static str,
    pub(crate) bold: &'static str,
    pub(crate) outline: &'static str,
    pub(crate) boxed: &'static str,
    pub(crate) pos_x: &'static str,
    pub(crate) pos_y: &'static str,
    pub(crate) scale: &'static str,
    pub(crate) rotation: &'static str,
    pub(crate) opacity: &'static str,
    pub(crate) volume: &'static str,
    pub(crate) mute: &'static str,
    pub(crate) unmute: &'static str,
    pub(crate) hide: &'static str,
    pub(crate) show: &'static str,
    pub(crate) export: &'static str,
    pub(crate) export_tip: &'static str,
    pub(crate) exporting: &'static str,
    pub(crate) exported: &'static str,
    pub(crate) export_failed: &'static str,
    pub(crate) cancel: &'static str,
    pub(crate) export_cancelled: &'static str,
}

static EN: Strings = Strings {
    ready: "Ready",
    switch_to: "中文",
    switch_tip: "切换到中文界面",
    joining: "Joining {}…",
    joined: "Joined {}",
    unreachable: "Cannot reach {}: {}",
    no_copy: "{}: the server has no copy ({})",
    done: "{}: done",
    sending: "Sending {}…",
    imported: "Imported {}",
    undo_tip: "Undo  (Ctrl+Z)",
    redo_tip: "Redo  (Ctrl+Shift+Z)",
    connecting: "Connecting",
    live: "Live",
    syncing: "Syncing",
    live_tip: "Edits here and by agents and other editors on this server meet in one project.",
    to_start: "To the start  (Home)",
    prev_frame: "Previous frame  (←)",
    play_tip: "Play / pause  (Space)",
    next_frame: "Next frame  (→)",
    to_end: "To the end  (End)",
    frame: "frame {}",
    split: "Split",
    split_tip: "Cut the selected clip at the playhead  (S)",
    delete: "Delete",
    delete_tip: "Remove the selected clip  (Delete)",
    title: "Title",
    title_tip: "Add a title at the playhead",
    new_title: "Title",
    colour: "Colour",
    colour_tip: "Add a colour clip at the playhead",
    drop_hint: "Drop video, pictures or sound to import",
    drop_overlay: "Drop to import at the playhead",
    fit_tip: "Fit the whole cut",
    zoom_in_tip: "Zoom in  (Ctrl + wheel)",
    zoom_out_tip: "Zoom out",
    snap_on: "Snapping on",
    snap_off: "Snapping off",
    gpu_preview: "GPU preview · {}",
    cpu_preview: "CPU preview",
    preview_failed: "Preview: {}",
    no_sound: "No sound: {}",
    select_hint: "Select a clip to edit it",
    shortcuts: "SHORTCUTS",
    keys: [
        ("Space", "Play / pause"),
        ("S", "Split at the playhead"),
        ("Delete", "Remove the clip"),
        ("Ctrl+Z", "Undo"),
        ("Ctrl+Shift+Z", "Redo"),
        ("← / →", "One frame"),
        ("Shift+← / →", "One second"),
        ("Ctrl+wheel", "Zoom the timeline"),
    ],
    kind_video: "Video",
    kind_picture: "Picture",
    kind_sound: "Sound",
    kind_title: "Title",
    kind_colour: "Colour",
    sec_text: "TEXT",
    sec_templates: "TEMPLATES",
    captions_imported: "{} captions from {}",
    sec_colour: "COLOUR",
    sec_transform: "TRANSFORM",
    sec_sound: "SOUND",
    sec_motion: "ANIMATION",
    anim_in: "In",
    anim_out: "Out",
    anim_none: "None",
    anim_length: "Length",
    anim_kinds: ["Fade", "Slide up", "Slide down", "Slide left", "Slide right", "Zoom in", "Zoom out", "Pop", "Spin"],
    anim_out_kinds: [
        "Fade out",
        "Slide down",
        "Slide up",
        "Slide right",
        "Slide left",
        "Shrink",
        "Grow",
        "Pop out",
        "Spin out",
    ],
    key_add: "Add a keyframe at the playhead",
    key_remove: "Remove the keyframe at the playhead",
    size: "Size",
    bold: "Bold",
    outline: "Outline",
    boxed: "Box",
    pos_x: "Position X",
    pos_y: "Position Y",
    scale: "Scale",
    rotation: "Rotation",
    opacity: "Opacity",
    volume: "Volume",
    mute: "Mute",
    unmute: "Unmute",
    hide: "Hide",
    show: "Show",
    export: "Export",
    export_tip: "Write an MP4 to {}",
    exporting: "Exporting {}/{}",
    exported: "Exported {}",
    export_failed: "Export: {}",
    cancel: "Cancel",
    export_cancelled: "Export cancelled",
};

static ZH: Strings = Strings {
    ready: "就绪",
    switch_to: "EN",
    switch_tip: "Switch to English",
    joining: "正在连接 {}…",
    joined: "已加入 {}",
    unreachable: "连不上 {}：{}",
    no_copy: "{}：服务器上没有这个文件（{}）",
    done: "{}：完成",
    sending: "正在上传 {}…",
    imported: "已导入 {}",
    undo_tip: "撤销  (Ctrl+Z)",
    redo_tip: "重做  (Ctrl+Shift+Z)",
    connecting: "连接中",
    live: "协作中",
    syncing: "同步中",
    live_tip: "你在这里的编辑，和 agent、其他编辑器在这个服务器上的编辑，汇到同一个工程里。",
    to_start: "跳到开头  (Home)",
    prev_frame: "上一帧  (←)",
    play_tip: "播放 / 暂停  (空格)",
    next_frame: "下一帧  (→)",
    to_end: "跳到结尾  (End)",
    frame: "第 {} 帧",
    split: "分割",
    split_tip: "在播放头处切开选中的片段  (S)",
    delete: "删除",
    delete_tip: "删除选中的片段  (Delete)",
    title: "文字",
    title_tip: "在播放头处添加文字",
    new_title: "标题",
    colour: "纯色",
    colour_tip: "在播放头处添加纯色片段",
    drop_hint: "把视频、图片或音频拖进来即可导入",
    drop_overlay: "松开即导入到播放头处",
    fit_tip: "整段显示",
    zoom_in_tip: "放大  (Ctrl + 滚轮)",
    zoom_out_tip: "缩小",
    snap_on: "吸附：开",
    snap_off: "吸附：关",
    gpu_preview: "GPU 预览 · {}",
    cpu_preview: "CPU 预览",
    preview_failed: "预览出错：{}",
    no_sound: "没有声音：{}",
    select_hint: "选中一个片段来编辑",
    shortcuts: "快捷键",
    keys: [
        ("空格", "播放 / 暂停"),
        ("S", "在播放头处分割"),
        ("Delete", "删除片段"),
        ("Ctrl+Z", "撤销"),
        ("Ctrl+Shift+Z", "重做"),
        ("← / →", "前后一帧"),
        ("Shift+← / →", "前后一秒"),
        ("Ctrl+滚轮", "缩放时间线"),
    ],
    kind_video: "视频",
    kind_picture: "图片",
    kind_sound: "音频",
    kind_title: "文字",
    kind_colour: "纯色",
    sec_text: "文字",
    sec_templates: "花字",
    captions_imported: "已导入 {} 条字幕（{}）",
    sec_colour: "颜色",
    sec_transform: "变换",
    sec_sound: "声音",
    sec_motion: "动画",
    anim_in: "入场",
    anim_out: "出场",
    anim_none: "无",
    anim_length: "时长",
    anim_kinds: ["渐显", "向上滑入", "向下滑入", "向左滑入", "向右滑入", "放大", "缩小", "弹出", "旋转"],
    anim_out_kinds: [
        "渐隐",
        "向下滑出",
        "向上滑出",
        "向右滑出",
        "向左滑出",
        "缩小消失",
        "放大消失",
        "缩回",
        "旋转消失",
    ],
    key_add: "在播放头处添加关键帧",
    key_remove: "删除播放头处的关键帧",
    size: "字号",
    bold: "粗体",
    outline: "描边",
    boxed: "底框",
    pos_x: "位置 X",
    pos_y: "位置 Y",
    scale: "缩放",
    rotation: "旋转",
    opacity: "不透明度",
    volume: "音量",
    mute: "静音",
    unmute: "取消静音",
    hide: "隐藏",
    show: "显示",
    export: "导出",
    export_tip: "把 MP4 写到 {}",
    exporting: "导出中 {}/{}",
    exported: "已导出 {}",
    export_failed: "导出失败：{}",
    cancel: "取消",
    export_cancelled: "已取消导出",
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn languages_are_found_from_locale_codes() {
        assert_eq!(Lang::from_code("zh-CN"), Some(Lang::Zh));
        assert_eq!(Lang::from_code("zh_Hans_CN"), Some(Lang::Zh));
        assert_eq!(Lang::from_code("en-GB"), Some(Lang::En));
        assert_eq!(Lang::from_code("fr-FR"), None);
    }

    #[test]
    fn templates_fill_in_order() {
        assert_eq!(fill(ZH.exporting, &[&3, &90]), "导出中 3/90");
        assert_eq!(fill(EN.unreachable, &[&"host", &"refused"]), "Cannot reach host: refused");
        assert_eq!(fill("{} and {}", &[&1]), "1 and ");
    }

    #[test]
    fn default_track_names_read_in_chinese() {
        assert_eq!(Lang::Zh.track_name("Video 2"), "视频 2");
        assert_eq!(Lang::Zh.track_name("B-roll"), "B-roll");
        assert_eq!(Lang::En.track_name("Video 2"), "Video 2");
        assert_eq!(Lang::Zh.command("clip.split"), "分割");
    }

    /// Every template has as many `{}` in both languages.
    #[test]
    fn templates_agree() {
        let count = |s: &str| s.matches("{}").count();
        for (en, zh) in [
            (EN.joining, ZH.joining),
            (EN.joined, ZH.joined),
            (EN.unreachable, ZH.unreachable),
            (EN.no_copy, ZH.no_copy),
            (EN.done, ZH.done),
            (EN.sending, ZH.sending),
            (EN.imported, ZH.imported),
            (EN.frame, ZH.frame),
            (EN.gpu_preview, ZH.gpu_preview),
            (EN.preview_failed, ZH.preview_failed),
            (EN.no_sound, ZH.no_sound),
            (EN.export_tip, ZH.export_tip),
            (EN.exporting, ZH.exporting),
            (EN.exported, ZH.exported),
            (EN.export_failed, ZH.export_failed),
        ] {
            assert_eq!(count(en), count(zh), "{en} / {zh}");
        }
    }
}
