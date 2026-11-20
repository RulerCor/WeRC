use super::*;
use rust_xlsxwriter::{Format, Workbook, XlsxError};

fn failed(error: XlsxError) -> AppError {
    match error {
        XlsxError::MaxStringLengthExceeded => AppError::new(
            "DIRECTORY_EXPORT_CELL_LIMIT",
            "一个字段超过Excel单元格的长度限制，请使用JSON导出；未写入文件",
        ),
        _ => AppError::new("DIRECTORY_EXPORT_FAILED", "无法生成Excel文件；未写入文件"),
    }
}
pub(super) fn render(rows: &[DirectoryRow]) -> AppResult<Zeroizing<Vec<u8>>> {
    table(
        "通讯录与群成员",
        &[
            "记录类型",
            "来源行ID",
            "账号",
            "昵称",
            "备注",
            "微信号",
            "本地类型",
            "删除标记",
            "认证标记",
            "群成员ID",
            "群内昵称",
            "本地关系标记",
            "公众号类型原码",
            "本地分类",
        ],
        rows.iter().map(|row| Vec::from(fields(row))),
        &[
            "数据来自当前选择的本地通讯录或群成员缓存，不代表服务器完整实时名单。",
            "跨页分别读取，不是同一时刻的快照；没有解析出的字段保持空值。",
            "头像导出数量：0。账号和来源ID均按文本保存，不会按公式执行或舍入长数字。",
            "本地分类根据缓存类型、关系标记与公众号类型生成；历史/非好友是本地线索，不代表服务器当前好友关系。缺失或冲突字段保持未知。",
        ],
    )
}
pub(crate) fn table(
    title: &str,
    labels: &[&str],
    rows: impl Iterator<Item = Vec<String>>,
    notes: &[&str],
) -> AppResult<Zeroizing<Vec<u8>>> {
    let mut book = Workbook::new();
    let sheet = book.add_worksheet();
    sheet.set_name(title).map_err(failed)?;
    sheet.set_freeze_panes(1, 0).map_err(failed)?;
    sheet
        .set_column_range_width(0, labels.len() as u16 - 1, 24)
        .map_err(failed)?;
    let header = Format::new().set_bold().set_background_color("#E7EEF8");
    for (column, label) in labels.iter().enumerate() {
        sheet
            .write_string_with_format(0, column as u16, *label, &header)
            .map_err(failed)?;
    }
    let mut count = 0;
    for (index, values) in rows.enumerate() {
        count = index + 1;
        let values = Zeroizing::new(values);
        for (column, value) in values.iter().enumerate() {
            // Explicit strings preserve full IDs; formulas and URLs remain literal text.
            sheet
                .write_string(index as u32 + 1, column as u16, value.as_str())
                .map_err(failed)?;
        }
    }
    sheet
        .autofilter(0, 0, count as u32, labels.len() as u16 - 1)
        .map_err(failed)?;
    let note_sheet = book.add_worksheet();
    note_sheet.set_name("导出说明").map_err(failed)?;
    note_sheet.set_column_width(0, 100).map_err(failed)?;
    for (row, line) in notes.iter().enumerate() {
        note_sheet
            .write_string(row as u32, 0, *line)
            .map_err(failed)?;
    }
    let bytes = Zeroizing::new(book.save_to_buffer().map_err(failed)?);
    if bytes.len() > MAX_BYTES {
        return Err(limit());
    }
    Ok(bytes)
}
