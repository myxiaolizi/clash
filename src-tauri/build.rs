fn main() {
    // Tauri 需要在编译期生成 context（图标、manifest、权限清单）
    tauri_build::build()
}
