# DBMind 数据库测试实验室

服务器 `192.168.2.127`（Ubuntu 22.04，32 vCPU / 62G 内存 / 400G 磁盘），用 Docker 跑 DBMind 支持的各数据库。
所有容器 `restart=unless-stopped`，宿主重启后自动拉起。

## 一、连接信息总表

主机一律填 `192.168.2.127`（下表的端口即宿主端口）。

| # | DBMind 类型 | 容器 | 端口 | 用户名 | 口令 | 库 / 服务名 | 备注 |
|---|---|---|---|---|---|---|---|
| 1 | MySQL | `lab-mysql` | 3306 | `root` | `Dbmind@123` | `demo` | 演示表 `users`(10 行) |
| 2 | MariaDB | `lab-mariadb` | **3307** | `root` | `Dbmind@123` | `demo` | 演示表 `products`(5 行) |
| 3 | PostgreSQL | `lab-postgresql` | 5432 | `postgres` | `Dbmind@123` | `demo` | 演示表 `orders`(20 行) |
| 4 | Redis | `lab-redis` | 6379 | 留空 | `Dbmind@123` | 库编号留空 | 演示键 `demo:*`，已开 AOF |
| 5 | MongoDB | `lab-mongodb` | 27017 | 无认证 | — | `demo` | 集合 `users`(3 条) |
| 6 | Elasticsearch | `lab-elasticsearch` | 9200 | 无认证 | — | 索引 `demo-products` | HTTP 协议，单节点 |
| 7 | SQL Server | `lab-sqlserver` | 1433 | `sa` | **`Dbmind@12345`** | `demo` | 表 `dbo.employees`(5 行) |
| 8 | Oracle | `lab-oracle` | 1521 | `system` 或 `dbmind` | `Dbmind@123` | **服务名 `FREEPDB1`**（不是 SID） | `dbmind.DEMO_ORDERS`(10 行) |
| 9 | DB2 | `lab-db2` | 50000 | `db2inst1` | `Dbmind@123` | **`DEMO`**（大写） | 首次启动约 10 分钟 |
| 10 | ClickHouse | `ch1` / `ch2` / `ch3` | 8123 / 8124 / 8125（HTTP）<br>9001 / 9002 / 9003（原生 TCP） | `default` | `Dbmind@123` | `demo` | 3 分片集群 `dbmind_cluster`，任选一节点连 |
| 11 | Doris | `doris-fe1` / `fe2` / `fe3` | 9030 / 9031 / 9032（MySQL 协议）<br>8030 / 8031 / 8032（Web UI） | `root` | `Dbmind@123` | `demo` | 3 FE + 3 BE，任选一个 FE 连 |

补充说明：

- **ClickHouse**：浏览器打开 `http://192.168.2.127:8123/play` 可直接查；驱动走 HTTP 口，JDBC 里要带 `result_overflow_mode=break`。
- **Doris**：类型一定选 **Doris**（不是 MySQL）；Web UI `http://192.168.2.127:8030`。别名别写 `rows`（关键字）。
- **Oracle**：口令含 `@`，在 shell 里用 sqlplus 时连接串要整体加引号（DBMind 界面不受影响）。
- **SQL Server**：sa 口令是 `Dbmind@12345`（带 5 位数字尾），别和别的库混了。
- **文件型库（SQLite / DuckDB / H2 / Derby）**：DBMind 连的是本机文件路径，**不在服务器上**，用跑 DBMind 的 Windows 机器任意 `.db` / `.duckdb` 文件即可。

## 二、数据目录 · 日志 · 重启

数据目录都在宿主机上（bind mount，直接 `tar` 就是备份）；日志多数走容器 stdout（`docker logs` 看）。

| 容器 | 数据目录（宿主机） | 日志位置 | 重启 |
|---|---|---|---|
| `lab-mysql` | `/opt/dbmind-labs/single/data/mysql` | `docker logs lab-mysql`（错误日志也在这里，容器内 `log_error=stderr`） | `docker restart lab-mysql` |
| `lab-mariadb` | `…/single/data/mariadb` | `docker logs lab-mariadb` | `docker restart lab-mariadb` |
| `lab-postgresql` | `…/single/data/postgresql` | `docker logs lab-postgresql` | `docker restart lab-postgresql` |
| `lab-redis` | `…/single/data/redis` | `docker logs lab-redis` | `docker restart lab-redis` |
| `lab-mongodb` | `…/single/data/mongodb` | `docker logs lab-mongodb` | `docker restart lab-mongodb` |
| `lab-elasticsearch` | `…/single/data/elasticsearch` | `docker logs lab-elasticsearch`；GC 日志在容器内 `/usr/share/elasticsearch/logs/gc.log` | `docker restart lab-elasticsearch` |
| `lab-sqlserver` | `…/single/data/sqlserver` | 容器内 `/var/opt/mssql/log/errorlog`（宿主机 `…/data/sqlserver/log/errorlog`） | `docker restart lab-sqlserver` |
| `lab-oracle` | `…/single/data/oracle` | `docker logs lab-oracle`；告警日志在容器内 `/opt/oracle/diag/rdbms/free/FREE/trace/alert_FREE.log` | `docker restart lab-oracle` |
| `lab-db2` | `…/single/data/db2` | `…/data/db2/config/db2inst1/sqllib/db2dump/DIAG0000/db2diag.log` | `docker restart lab-db2` |
| `ch1` `ch2` `ch3` | `…/clickhouse/data/ch1` `ch2` `ch3` | `docker logs ch1`；容器内 `/var/log/clickhouse-server/clickhouse-server.log` | `docker restart ch1` |
| `ch-keeper1..3` | `…/clickhouse/data/keeper1..3` | `docker logs ch-keeper1` | `docker restart ch-keeper1` |
| `doris-fe1..3` | `…/doris/data/fe1..3` | `docker logs doris-fe1`；容器内 `/opt/apache-doris/fe/log/fe.log`（`fe.audit.log` 审计） | `docker restart doris-fe1` |
| `doris-be1..3` | `…/doris/data/be1..3` | `docker logs doris-be1`；容器内 `/opt/apache-doris/be/log/be.INFO` | `docker restart doris-be1` |

`…` = `/opt/dbmind-labs`。三套 compose 项目：

| 项目 | 目录 | 整组操作 |
|---|---|---|
| `dbmind-single` | `/opt/dbmind-labs/single` | `docker compose ps \| stop \| start` |
| `dbmind-clickhouse` | `/opt/dbmind-labs/clickhouse` | 同上 |
| `dbmind-doris` | `/opt/dbmind-labs/doris` | 同上 |

> 容器重建（`compose down` 后再 `up`）会丢的只有**没挂载的东西**：
> ClickHouse 的 `/var/log/clickhouse-server/*`、Doris 的 `fe/log` `be/log`、Oracle 的 `/opt/oracle/diag`。
> 数据本身都在上表的宿主机目录里，不会丢。

## 三、两个集群的形态

**ClickHouse**：3 个数据节点（`ch1/ch2/ch3`）+ 3 个 Keeper（Raft 仲裁）。集群名 `dbmind_cluster`，
**3 分片 × 1 副本**；内部网络 `172.28.0.0/16`（ch1-3 = .11/.12/.13，keeper = .21/.22/.23）。

```bash
docker exec ch1 clickhouse-client --password 'Dbmind@123' -q \
  "SELECT hostName(), count() FROM clusterAllReplicas('dbmind_cluster', demo.events_local) GROUP BY hostName()"
```

**Doris**：3 FE（fe1 是 Master）+ 3 BE；内部网络 `172.29.0.0/16`（fe = .11/.12/.13，be = .14/.15/.16）。

```bash
docker exec doris-fe1 mysql -uroot -p'Dbmind@123' -P9030 -h127.0.0.1 -e "SHOW FRONTENDS\G" | grep -E "Host|Role|Alive"
docker exec doris-fe1 mysql -uroot -p'Dbmind@123' -P9030 -h127.0.0.1 -e "SHOW BACKENDS\G"  | grep -E "Host|Alive"
```

演示数据：`demo.events` 4000 行散在 3 个分片；Doris `demo.events` 3000 行、副本数 3。

## 四、常用操作

```bash
# 全量状态 / 资源
docker ps --format 'table {{.Names}}\t{{.Status}}\t{{.Ports}}'
docker stats --no-stream

# 改配置后重载（单机库为例）
cd /opt/dbmind-labs/single && docker compose up -d mysql

# 进容器
docker exec -it lab-mysql bash

# 备份（不用停库）
tar -czf ~/lab-$(date +%F).tgz -C /opt/dbmind-labs single/data clickhouse/data doris/data

# 重置单个库（连数据一起清掉）
cd /opt/dbmind-labs/single && docker compose rm -sf mysql && rm -rf data/mysql && docker compose up -d mysql

# 重置整组集群
cd /opt/dbmind-labs/doris && docker compose down && rm -rf data && docker compose up -d
```

## 五、连接时的常见坑

| 现象 | 原因 / 处理 |
|---|---|
| Oracle 连不上、报监听/服务名错误 | 服务名填 `FREEPDB1`，不是 SID；端口 1521 |
| SQL Server 登录失败 | 口令是 `Dbmind@12345`（比别的库多两位数字） |
| Doris 报字符集/握手错（如 `Unknown character set`） | 类型选 Doris，驱动用 mysql-connector-j 8.x+（DBMind 已内置 9.x） |
| ClickHouse 查询报 `TOO_MANY_ROWS_OR_BYTES` | 驱动 JDBC URL 需要 `result_overflow_mode=break` |
| Redis 报 NOAUTH | 必须填口令 `Dbmind@123`（没有用户名） |
| ES 连不上 | 用 9200 HTTP，不是 9300；无账号口令 |
| MongoDB 认证失败 | 本实例**没开认证**，用户名口令留空 |
| DB2 连不上 / 库不存在 | 库名 `DEMO`；首次启动要等 10 分钟左右才建好 |
