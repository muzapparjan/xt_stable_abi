# 演示：一个内核、一份发布接口，以及三个程序

**中文** · [English](README.md)

这个目录装着本 ABI crate 自己的测试替代不了的那些 crate，因为它们是一个真实项目，
不是夹具：

| crate | 路径 | 依赖 | 构建出 |
| --- | --- | --- | --- |
| `xt_kernel` | `kernel/` | 无 | 一个 `rlib` |
| `xt_kernel_api` | `api/` | `xt_stable_abi` | 一个 `rlib` |
| `xt_kernel_impl` | `impl/` | `xt_kernel`、`xt_kernel_api` | 一个 `rlib` |
| `xt_kernel_dynlib` | `dynlib/` | `xt_kernel_impl`、`xt_kernel_api` | **只有**一个 `cdylib` |
| `xt_kernel_app` | `app/` | `xt_kernel_api` | 一个可执行文件 |
| `xt_kernel_static` | `static/` | `xt_kernel_impl`、`xt_kernel_api` | 一个可执行文件 |
| `xt_kernel_native` | `native/` | `xt_kernel` | 一个可执行文件 |

另有两个 crate 装着这些程序共用的东西，好让对照发生在**路径**之间，而不是在工作负载
之间：

| crate | 路径 | 装什么 |
| --- | --- | --- |
| `xt_bench` | `bench/` | 输入、期望答案、计时轮数与检查报告 |
| `xt_surface_checks` | `surface-checks/` | 两个边界程序都跑的场景清单 |

上面那张表写出的规则就是这个布局的意义：

- 内核不依赖本 ABI crate，它里面没有任何地方提到稳定布局、descriptor 或动态库；
- 接口层是**在内核之后**写的，由它决定什么能离开内核：内核借用的 token、带计数的
  map、排序堆与错误枚举留在里面，调用方拿到的是文本、数字、元组、序列、一个时间与
  一个 `AbiError`；
- 实现依赖两者，自己什么也不加：每个方法转发给内核，并把答案复制进那些发布类型；
- 动态库依赖实现，自己同样什么也不加：它导出两个名字，并调用进实现；
- `xt_kernel_app` 只依赖接口层，所以内核不在它的二进制里；`xt_kernel_static` 加上
  了实现，好让它不需要库文件也能调用同一个入口点。

### 为什么实现是一个单独的 crate

实现必须以两种形态存在：一次作为 `rlib`，好让程序链接它、在没有库文件的情况下跑同一
套协议；一次作为 `cdylib`，好让宿主在运行期加载它。

把两者放进一个 crate，会让交付物同时产出一个 `rlib`。那是泄露，不是细节：`rlib`
里装着 `.rmeta`，而它保存源码级信息，包括文档注释。所以拆成两个——`xt_kernel_impl`
构建那个 `rlib`，`xt_kernel_dynlib` 声明 `crate-type = ["cdylib"]`，里面是两个转发
函数。于是打包构建目录也不会把交付物自己的元数据交出去。

交付物是动态库，别的都不是。[`boundary-guidance.zh-CN.md`](../docs/boundary-guidance.zh-CN.md)
写明这条规则和它依据的实测数据。

## 三个程序

同样的场景、同样的输入、同样的期望答案，三种到达内核的方式：

| 程序 | 链接什么 | 调用跨过什么 |
| --- | --- | --- |
| `xt_kernel_native` | `xt_kernel` | 什么也不跨：一次普通的函数调用 |
| `xt_kernel_static` | `xt_kernel_impl` 作为库、`xt_kernel_api` | 那些发布的表，被链接进来 |
| `xt_kernel_app` | `xt_kernel_api` | 那些发布的表，在一个运行期打开的库里 |

`xt_kernel_static` 调用的正是 `xt_kernel_app` 用 `GetProcAddress`/`dlsym` 查出来的
那个入口点，所以调用之后的协议完全相同：descriptor、adoption，然后每个访问器都走
同一批表。两个边界程序之间唯一的差别，是这些表放在哪里。

把实现链接进来**不会**把调用变成直接调用。两个边界程序都会在对象的表里查一个槽位、
经过存在那里的函数指针调用、并收到一个通过宿主分配器分配的 owned 边界值。链接省下
的是那个文件和两次符号查找，而且在启动时只有一次——所以两个边界程序在每次调用上都
一致，只有参考程序更便宜。[`boundary-guidance.zh-CN.md`](../docs/boundary-guidance.zh-CN.md)
给出数字。

所以把三份输出并排放在一起，同时隔离了两件事：

- 边界协议本身的成本：把 `xt_kernel_static` 与 `xt_kernel_native` 相比（`split_fixed`
  那一行把跨越本身——约 7 ns——与构造一个返回的 owned 值——约 30~40 ns——分开）；
- 通过一个文件到达边界的成本：把 `xt_kernel_app` 与 `xt_kernel_static` 相比（实测：
  在噪声范围内）。

检查按名字对齐。边界程序多做四项参考实现做不到的检查——descriptor 的名字、它的
版本、协议版本、以及 shutdown 钩子——并且用另一种方式表述 panic 检查，因为在参考
实现里 panic 在同一进程内被抓住，而在边界程序里它在库那一侧被兜住并作为一个状态
回报。

## 运行它们

```sh
cargo build --workspace
cargo run -p xt_kernel_native
cargo run -p xt_kernel_static
cargo run -p xt_kernel_app
```

`cargo run` 会把库放在可执行文件旁边，那正是 `xt_kernel_app` 首先找的地方；一次
部署也是这样。也可以把路径作为第一个参数传进去：

```sh
cargo run -p xt_kernel_app -- target/debug/xt_kernel_dynlib.dll
```

每个程序为每项检查打印一行、为每个计时打印一行，最后打印计数。检查失败时它以非零
退出码结束，所以每一个都可以当 CI 的一步。

## 可以发布什么

只有动态库：`target/release/xt_kernel_dynlib.dll`（或 `.so`、`.dylib`）。

```powershell
Get-ChildItem -Recurse <bundle> -Include *.rlib,*.rmeta,*.pdb,*.d
```

这条查询必须返回空。一次完整构建之后的 `target/release` 会返回 54 个这样的文件，其中
包括 12 个 `rlib`；它们属于那些源码不该离开构建机的 crate。
[`boundary-guidance.zh-CN.md`](../docs/boundary-guidance.zh-CN.md) 解释原因，并写明
这条规则的其余部分。

## 它证明了什么，没证明什么

这些程序展示：每一种发布形状的值都能双向走完这一趟——字符串、序列、元组、带错误的
标记结果、由另一个句柄返回并再传进去的生成句柄、在库那一侧被兜住的 panic，以及插件
被丢弃时的关停顺序。

有一个答案不可能在三条路径上复现：一个通过发布句柄吸收对端的会话，学到了对端的名字
与总数，但没有学到它的文本，所以它收尾那一行数到两段文本，而参考实现数到三段。词数
是相同的，而那正是发布接口所承诺的。

它们不拿恶意输入测内核，不用分析器量吞吐，也不在第二个目标上跑这套接口。那些属于
随后的评审，而这些程序是它的量具。
