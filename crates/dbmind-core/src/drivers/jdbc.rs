// 本文件已废弃：JDBC 专用驱动已提升为通用的 `agent_driver`（同时承载 JDBC 与
// MongoDB 等原生协议宿主），实现见同目录 agent_driver.rs。
//
// 保留此文件仅因当前沙箱不允许删除工作区外的文件；它**没有**被 mod.rs 声明为模块，
// 因此不参与编译。可安全删除：
//   Remove-Item crates\dbmind-core\src\drivers\jdbc.rs
