use mlua::prelude::*;
use nvim::Vim;

#[mlua::lua_module]
fn module_name(lua: &Lua) -> LuaResult<LuaTable> {
    let exports = lua.create_table()?;
    exports.set(
        "hello",
        lua.create_function(|lua, ()| {
            Vim::from(lua).notify("\nHello World", nvim::LogLevel::Info, None);
            Ok(())
        })?,
    )?;
    Ok(exports)
}
