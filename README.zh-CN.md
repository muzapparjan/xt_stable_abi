# xt_stable_abi

**中文** · [English](README.md)

在**两个各自独立构建的 Rust 二进制**之间交换值所需的类型、所有权与虚表。

由不同编译器版本、不同 feature 统一结果、不同依赖图构建出来的两个程序，不共享
Rust ABI。本 crate 给它们一个：跨边界的每个值都有声明好的字节布局，每个拥有所有权
的分配都带着产出它的那套分配器，每次动态调用都经过一张头几个字段就说明「这张表是
什么」的表。

三条运行层面的事——边界在百万级调用下的代价、发布出去的库会暴露多少实现、以及交付
时该发什么文件——收在
[`docs/boundary-guidance.zh-CN.md`](docs/boundary-guidance.zh-CN.md) 里，连同它们依据的实测数据。

第三条是硬规则，这里再写一遍：**只发动态库。** `rlib` 里装着 `.rmeta`，它保存的是
源码级信息，包括文档注释；`.pdb` 里装着全量 Rust 符号名。两者都不能发布，构建目录
也不能整体打包。

## 规则

- **布局显式。** 边界类型都是 `#[repr(C)]`，每个宽度都写出来（`u64`、`u32`），
  不从 `usize` 取。
- **标记显式。** `Option` 与 `Result` 在某个编译器下会和 niche 共用字节；这里的
  类型以一个 `u32` tag 开头，空值的载荷存储被清零，所以空值不会带上调用方栈上的
  内容。编译器在字段之间插入的填充字节不写：它们不是值的一部分，而值也不能按原始
  字节比较或存储。
- **拥有所有权的值带着它的分配器。** `StableString`、`StableVec` 这些类型存一个
  指针、一个长度、一个容量和一个 [`AbiAllocator`]。接收侧丢弃一个值时，调用的是
  产出那块内存的函数，所以一侧可以把内存交给使用另一套分配器的宿主。
- **表头说明这份声明是什么。** 生成的表以 `type_id`、`signature`、
  `layout_revision`、`size`、`align`、随后表项的个数和一个 drop 函数开头，后面
  每个方法一项。每个表项带着它要调用的方法的标识符，所以接收方是比较数字找到自己
  要的槽位，而不是相信某个位置，并且最多只读表头记下的那些项。本 crate 生成的表按
  这个标识符有序，所以长表上的一次查找是二分；短表逐个走，手写的表在未命中之后
  接着走一遍。由另一份声明构建的表，或者按另一套边界布局构建的表，会被拒绝，而不
  会从错误的槽位调过去。签名折叠了这次构建编译进去的那些方法的标识符，所以被
  `#[cfg]` 去掉一个方法就会改变这个数。方法提到的每个类型有自己的数，
  [`AbiIdentity::IDENTIFIER`]，而方法的标识符是由它提到的那些类型的数折叠成的，
  不是由它们的写法折叠成的：`StableStr`、`StableStr<'_>` 与
  `xt_stable_abi::StableStr` 是同一个类型，所以它们给同一个标识符，边界类型的
  本地别名也指向那个类型。本 crate 之外的类型要实现 [`AbiIdentity`] 才能参与——
  `unsafe_stable_identity!` 为某个名字写出这个实现——而容器折叠进去的那些种类，
  对于手写的表来说是协议的一部分。这个折叠是可交换的：方法列出的顺序不是声明的一
  部分，因为一次调用是靠比较标识符找到槽位的，不是靠数表项。
- **调用只能声明边界类型。** 参数与返回类型必须实现 [`AbiType`]；声明里写
  `String`、`usize` 或 `bool` 编译不过。返回的值在访问器里由 [`AbiType::validate`]
  检查之后才交给安全代码：tag 不在该类型定义的集合里、文本不是 UTF-8、向量的长度
  超过容量、map 的键没有按序、对象的表与地址不符，都在那里被拒。参数也会被检查，
  只是由能上报的那一侧检查：`stable_object!` 生成的表的访问器在调用前逐个检查参数，
  不合法就丢弃该值并 panic，因为那张表没有状态字可以上报拒绝；`stable_object_guarded!`
  生成的表在它的 trampoline 里、实现看到参数之前检查，并回报
  [`AbiStatus::InvalidArgument`]。
- **被兜住的 panic 随这次调用一起走。** `stable_object_guarded!` 从调用方把一个状态
  字传进 trampoline，所以在实现位于另一个动态库、每个线程局部变量都是各自一份副本
  的情况下，这份报告仍然到达得了。这个字区分两种失败：panic 是
  [`AbiStatus::Panic`]，而接收侧拒绝的参数是 [`AbiStatus::InvalidArgument`]，它说明
  实现根本没被进入。被拒的参数是被**丢弃**而不是被释放的，因为它的字节并不描述一个
  接收侧可以交还的所有者；通过检查的那些参数正常释放，丢弃它们会让发送方交出来的
  东西泄漏。按值接收对象的方法保留着那个接收者，由它自己的 drop 释放。panic hook
  仍然会跑：`catch_unwind` 停下的是展开，不是默认 hook，所以一个不能把自己的 panic
  文案写进进程标准错误流的实现，要自己装一个 hook。
- **一个方法可以比它对面那一侧旧。** 严格 adoption 要求声明里的每个方法都在。
  `abi_try_from_stable_compatible` 接受这样一份方法列表：它是本声明的一个修订——
  声明同一个方法集合的子集或超集，且没有方法被写两次。与它不共享整份声明的列表，
  或者把一个方法写了两次的列表，属于另一个 trait 或者挪动过某个方法，会被拒绝，而
  不会被调过去。
- **版本是协商出来的，不是假设的。** 一个 [`AbiVersion`] 服务由同一 major 版本、
  相等或更旧的 minor 版本提出的请求。所以宿主接受 newer minor 的库——决定字段能不
  能读的是 major 数与布局修订号——并拒绝比它要求的版本更旧的库。

## 方法的形状

`stable_object!` 与 `stable_object_guarded!` 接受：

- `fn name(&self)`、`fn name(&mut self)` 和 `fn name(self)`；
- 接收者后面任意多个 `参数: 类型`，末尾可以有逗号；
- 可选的 `-> Type`，其中 `-> ()` 与不写返回类型是同一份声明；
- 结尾的 `;`，或花括号里的默认实现。默认实现是 trait 的一部分，所以不覆盖它的实现
  仍然通过槽位应答；
- 方法上的属性，包括 `#[cfg]`：trait 项、表项、访问器与 trampoline 一起被门控。

泛型方法与关联类型或关联常量命名的是这个形状承载不了的东西；它们在**声明处**就被
报出来，报错文案会说这件事。把接收者的生命周期写出来——`fn peek<'a>(&'a self)`——
就是同一个泛型方法，也在那里被拒：接收者只有 `&self`、`&mut self`、`self` 三种写法。
返回类型里带生命周期，以及 `-> Self`，同样在那里被拒：槽位只为这次调用出借它的值，
所以返回的借用会活得比存储更久，而 `Self` 是实现类型，调用方不知道它。`'static` 按
同一条规则被拒：动态库的存储在库卸载之后就不在了，而类型系统不知道卸载这件事。改成
返回拥有所有权的值。嵌在返回类型里的借用类型（`StableOption<StableStr>`）也按同一条
规则被拒，因为它命名的那个借用正是槽位借不出来的。

声明也不能命名 supertrait。表列的是所声明 trait 的方法，所以继承来的方法没有对应
的槽位；要么在本 trait 里重写继承来的方法，要么用一个自身声明列出了这些方法的对象
跨边界。宏在声明处就把这种形状报出来，而不是等到后来在一个编译通过但答不了一次调用
的 trait 上失败。

只在参数类型上不同的方法，写成每个实例化一个 trait 方法，泛型那个留在实现上
（`tests/objects.rs` 里有这个形状）：

```rust
stable_object! {
    pub struct ConverterObject;
    pub struct ConverterVTable;
    pub trait Converter {
        fn identity_u32(&self, value: u32) -> u32;
        fn identity_u64(&self, value: u64) -> u64;
    }
}

impl PassThrough {
    fn identity<T>(&self, value: T) -> T {
        value
    }
}

impl Converter for PassThrough {
    fn identity_u32(&self, value: u32) -> u32 {
        self.identity(value)
    }
    fn identity_u64(&self, value: u64) -> u64 {
        self.identity(value)
    }
}
```

## 类型

标量与它们的受检包装（[`StableBool`]、[`StableChar`]），文本（[`StableStr`]、
[`StableString`]、[`StableCowStr`]），序列（[`StableVec`]、[`StableSlice`]、
[`StableSliceMut`]、[`StableArray`]、[`StableCowSlice`]），[`StableBox`]，带标记
的值（[`StableOption`]、[`StableResult`]），有序集合（[`StableMap`]、
[`StableSet`]），按调用方给出的哈希排序的集合（[`StableHashMap`]、
[`StableHashSet`]），成组的值（[`StablePair`]、[`StableTriple`]、[`StableQuad`]），
文件名与路径（[`StableOsStr`]、[`StableOsString`]、[`StablePath`]、
[`StablePathBuf`]），时间（[`StableDuration`]、[`StableSystemTime`]、
[`StableInstant`]），条目的名字（[`Handle`]）与对象的名字（[`StableDyn`] 以及宏
生成的句柄）。领域枚举用 [`stable_enum!`] 声明，它把 tag 与载荷联合体写出来而不是
derive 出来；它也为这个类型写一个 `Wipe`，所以不能被擦除的载荷要用
[`stable_enum_unwiped!`]，它声明同一个枚举但不带 `Wipe`。声明里点名这个类型的值要
有哪些 trait——`derives = [Clone, PartialEq, Eq, PartialOrd, Ord, Hash, StableHash]`
——因为 `derive` 读的是联合体，而不是 tag 选中的那份载荷；`Clone` 同时写出
[`TryClone`]，`StableHash` 让这个类型能直接做本 crate 自己算数的哈希 map 的键，
而 `default = Variant;` 指名 `Default` 值是哪个变体，`Default` 也是靠它才打开。
每个拥有所有权的类型都带着产出它的那套分配器；每个有着编译器抓不住的状态的类型，
都在边界上校验这些状态。复制有两条路：`Clone` 给接受「分配失败就 panic」的调用方，
[`TryClone`] 给把失败报出来的调用方。`TryClone` 为标量包装与时间读数实现，为每个
拥有所有权的容器实现（包括哈希 map 与哈希 set，它们的副本保留存下的那些数），也为
借用视图实现（副本命名同一份存储）。可写视图没有实现，因为同一块存储的两个可写视图
会让两个调用方同时往里写；需要把它交出去的调用方 move 它。对象句柄也没有 `Clone`：
句柄拥有那个值，两个句柄会释放它两次。一侧需要同一个对象的第二个名字时，把句柄
传过去，或者用 [`Handle`] 命名它——持有那个值的表可以发出不止一个这样的名字。

哈希 map 按表项携带的那些数排序，所以产出这些数的函数是两端约定的一部分。由调用方
传入（[`StableHashMap::insert`] 及其同类）就把那个函数变成你的，约定也变成显式的：
接收侧对它的检查是 [`StableHashMap::validate_with`]。[`StableHash`] 是另一条路：
数由本 crate 计算，所以两端天然一致，[`StableHashMap::insert_stable`] 与
[`StableHashMap::validate_stable`] 之间不需要约定任何东西。

零尺寸元素的容器除了长度什么也没带，而那个数没有任何上界，所以边界拒绝装了元素的
这种容器：没有东西要带的发送方，把那个数当数字发出去。

排序默认是稳定归并，占一块与向量等大的临时块。[`StableVec::sort_unstable_by`]
重排的是向量自己已经拥有的那块：它不分配，所以没有 fallible 形式，代价是放弃比较
相等的元素之间的顺序。

## 示例

```rust
use xt_stable_abi::{stable_object, StableStr, StableString};

stable_object! {
    /// A handle to a greeting source.
    pub struct GreeterObject;
    /// The table behind `GreeterObject`.
    pub struct GreeterVTable;
    /// Produces greetings.
    pub trait Greeter {
        /// Returns a greeting for `name`.
        fn greet(&self, name: StableStr<'_>) -> StableString;
    }
}

struct Plain;

impl Greeter for Plain {
    fn greet(&self, name: StableStr<'_>) -> StableString {
        StableString::from_str(&format!("Hello, {}", name.as_str())).unwrap()
    }
}

let object = GreeterObject::abi_from_value(Plain);
let greeting = unsafe { object.greet(StableStr::from_str("Ada")) };
assert_eq!(greeting.as_str(), "Hello, Ada");
```

生成的访问器是 `unsafe` 的，因为它的正确性依赖于对面那一侧是由同一份声明构建的；
[`stable_object`] 记录了它接受的方法形状。

## 入口协议

宿主构造一个 [`HostServices`]（它自己的字节数、版本、分配器、日志汇、上下文和一个
查询函数），调用库导出的入口点。入口点返回一个 `u32` 状态码：本宿主不定义的状态码
是失败，绝不是非法的枚举值。库填一个 [`PluginDescriptor`]，它的第一个字段是一个
[`AbiHeader`]，带着宿主提供的缓冲容量与库实际写出的字节数。[`load`] 校验状态码、
字节数、布局修订号以及对象是否存在；[`LoadedPlugin`] 释放库拥有的东西，然后跑它的
shutdown 钩子，最后丢弃宿主提供的 keep-alive 值——那一点就是宿主可以卸载库的地方。

虚表按实现类型建一次、由该类型的每个值共享，所以它比所有值活得久，并且属于**进程**
的分配器，而不是构建它的那个模块的。一个可能被卸载的库在它的 shutdown 钩子里调用
[`release_cached_tables`] 把这些表交还回去；下一次加载会建自己的。这次调用承担卸载
本来就承担的义务：库交出去的每个对象都已释放，也没有线程正在经过它的表执行调用。

查询函数是分配器与日志汇之外一切的通道：库按标识符请求一个服务，再按两端约定的
布局读取答案。宿主对象、回调表、时钟都走这条路，而有不止一个对象的插件通过它交出
的那个对象（一个返回另一个 [`StableDyn`] 的访问器）把这些对象发布出去。

```rust,ignore
// host side
let services = HostServices::new(AbiAllocator::system());
let entry = /* obtained from the loaded library */;
let mut plugin = LoadedPlugin::new(unsafe { load(entry, &services, ABI_VERSION) }?);
let object = MyObject::abi_try_from_stable(plugin.take_api())?;
```

descriptor 把 `written` 记成写方产出的字节数，而 `MIN_DESCRIPTOR_SIZE` 是第一版
布局的冻结声明的大小，所以 descriptor 增加字段时它不会变大。按那一版布局构建的库，
会被后来长胖了结构的宿主接受，因为宿主提供的缓冲至少是它自己那版布局的大小，而
`written` 之后的每个字节都是宿主写的。

不成立的是相反的方向：库把 descriptor 长大。写方在写之前用自己那版布局检查缓冲容量
（`PluginDescriptor::store`），所以追加了字段的库需要一个缓冲覆盖得住它的宿主；按旧
布局构建的宿主答 `InvalidDescriptor`，这是入口点报出来的失败，而不是宿主有权做的
版本检查。所以追加 descriptor 字段的版本是两端必须达成一致的版本，不是 minor 号
自己就能带过去的一次改动。

光有字节数说不清字段的位置，所以头部还记着 `layout_revision`。按另一个修订号写出的
descriptor 以 `VersionMismatch` 被拒，而且因为它的字段可能落在别的偏移上，它既不被
释放也不被调过去——把自身长度说大了的 descriptor 得到的是同一个答案。来自另一个
major 版本的 descriptor 同样处理。newer minor 的库被接受；比宿主要求的版本更旧的库，
在返回错误之前它的字段被释放、钩子被跑过，因为它自己那份报告仍然是可信的，只是它
发布出来的东西用不了。

## 一个走通的例子：内核、接口层、库、程序

`demo/` 按真实项目构建它的顺序，搭出本 crate 存在的那个形状：

- `demo/kernel`（`xt_kernel`）是一个普通库——一个文本分析引擎——对本 crate 一无
  所知。它有自己的 31 项测试，并且广泛使用这门语言：借用迭代器、泛型堆、`dyn`
  trait、`Cell` 与 `Mutex`、递归、并查集、`const` 构建的 CRC 表、透明的位集合、
  带 `source` 的错误枚举，以及一张函数指针跳转表。
- `demo/api`（`xt_kernel_api`）是**从这个库出发**设计的：它命名外部调用方可以要求
  什么，并再导出边界类型。内核留给自己的东西——借用的 token、带计数的 map、排序堆、
  错误枚举——不跨边界。它还带着 loader，所以一个程序不需要自己写任何操作系统代码
  就能打开一个库。
- `demo/impl`（`xt_kernel_impl`）依赖内核与接口层，实现每一个发布出去的方法。它是
  一个库，所以同一份实现可以被链接进程序，也可以被编译成库。
- `demo/dynlib`（`xt_kernel_dynlib`）导出两个名字，并调用进 `xt_kernel_impl`。
  它只构建一个 `cdylib`，别的什么都不构建：交付物不能同时产出一个 `rlib`，那份
  元数据里装着源码级信息。
- `demo/app`（`xt_kernel_app`）只依赖接口层。它在自己可执行文件旁边找到库，加载
  它，然后对文本、字节、图、数字、构建、会话与关停跑那些场景。
- `demo/static`（`xt_kernel_static`）与 `demo/native`（`xt_kernel_native`）用另外
  两种方式跑同样的场景：对同一个入口点、实现被链接进二进制；以及对内核、完全没有
  边界。
- `demo/bench`（`xt_bench`）与 `demo/surface-checks`（`xt_surface_checks`）装着
  输入、期望答案、计时轮数与场景清单，所以三个程序的差别只在它们走的路径，别处没
  有差别。

构建并运行三条：

```sh
cargo build --workspace
cargo run -p xt_kernel_native
cargo run -p xt_kernel_static
cargo run -p xt_kernel_app
```

任何一个检查失败时，程序以非零退出码结束，并且它会打印每个场景花的时间，所以三条
可以并排放在一起：参考实现、链接进来的同一个边界、以及通过一个文件到达的同一个
边界。`demo/README.zh-CN.md` 写清每个 crate 守的依赖规则，以及参考实现与「只拿得到发布
接口的调用方」之间哪一个答案不同。

## 已验证的内容

- `tests/` 装着全部测试；`src/` 里没有。
- `tests/layout.rs` 用独立的 `#[repr(C)]` 镜像读活着的值，所以一个移动了的字段是
  让一条测试变红，而不是让下游的构建变红。
- `tests/end_to_end.rs` 加载两个动态库：一个按本 crate 构建，另一个
  （`plugins/legacy_plugin`）不依赖本 crate、用手写出那些边界类型。两个都用
  `LoadLibraryW`/`dlopen` 加载。同一个文件钉住只有动态库才能展示的那几件事：库内
  被兜住的 panic 仍然到达宿主、shutdown 钩子在对象释放之后才跑、手写的库拒绝一个
  它填不满的缓冲。
- `tests/table_release.rs` 构造一个对象、丢掉它，再调用
  [`release_cached_tables`]。一个包着系统分配器的全局分配器记录下表自己的地址被
  交还；第二次调用什么也不释放，下一个值会建一张新表。
- `demo/api/tests/unload.rs` 在一个进程里三次加载同一个库、使用它，并问操作系统
  这个模块在 drop 之前是否在映射中、drop 之后是否已经不在。第二条被忽略的测试量
  这些轮次留下的私有字节。
- `tests/hostile.rs` 把固定的畸形 descriptor、tag 与长度喂给接收侧，并断言每一个
  产生什么状态码。
- `tests/entry.rs` 覆盖 descriptor 协议，包括一个只认识第一版布局的写方、一个被拒
  但钩子仍然跑了的 descriptor，以及钩子与对象释放的先后顺序。
- `tests/validation.rs` 构造这样的值：它们的字节说着该类型没有定义的事情——用另一
  种载荷类型撑着的未定义 tag、tag 指向另一个字段的写时复制值、不是 UTF-8 的文本、
  长度超过容量的向量、有表但没有地址的句柄、表项比它记录的还多的表——并断言每一个
  都被拒绝并被释放，且不去碰它没有描述的字节。
- `tests/additions.rs` 走那些不是普通 getter 的操作：表项、draining、保留式修改、
  多余容量、时长算术、擦除容器，以及一次用 map、box、错误与时长作答的守卫式调用。
- `tests/surface.rs` 走一遍访问器与迭代器：填充与替换一个可选值、擦掉一次 `take`
  留下的东西、把 result 翻过来、借用并拆开一个 box、以三种形式迭代每一个集合、
  拆分与合并 map、比较 set 的关系、拆开一个路径、以及把时长相加。
- `tests/collections.rs` 与 `tests/vectors.rs` 单独覆盖这些集合：有序 map 与 set，
  哈希 map 与 set 包括它们存下的那些数，每一种排序与它取的块，插入与删除以及随之
  而来的析构，还有那些编辑用的辅助方法。
- `tests/enumeration.rs` 声明枚举并检查声明写出来的东西：tag、载荷、视图、擦除、
  标识符、声明用 `derives` 点名的 trait、一个无法在其上构造值的载荷，以及守卫式表
  答不出来的时候用的 fallback。
- `tests/objects.rs` 与 `tests/guarded.rs` 覆盖生成的表：每一种接收者形式与方法形
  状、默认实现、接收侧拒绝的参数、在边界处被兜住的 panic，以及按值消费对象的方法
  取走的那个接收者的释放。
- `tests/rfc3339.rs` 把时间点写成 RFC 3339 文本、读回来、按数字偏移换算，并拒绝那
  些没有指名任何有效时刻的写法。

## 运行

```sh
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --workspace
cargo run -p xt_kernel_native
cargo run -p xt_kernel_static
cargo run -p xt_kernel_app
```

这里的 `cargo test` 一次跑一条测试：`.cargo/config.toml` 设了
`RUST_TEST_THREADS = "1"`，因为有几个测试文件把它们的量具——释放次数、分配调用
次数、shutdown 钩子次数——放在 `static` 值里，两条测试同时写一个计数器会读到彼此
的工作。想要并行的运行传 `-- --test-threads <n>`。

没有 `std` 时，在一个没有 `std` 可以回退的目标上检查：

```sh
rustup target add wasm32-unknown-unknown
cargo check -p xt_stable_abi --no-default-features --target wasm32-unknown-unknown
```

单独一条 `cargo test` 不构建动态库，所以端到端测试在它们缺失或比源码旧时自己构建。
测试从自己所在的路径定位 profile 目录，所以 `CARGO_TARGET_DIR` 与 `--release` 都
能用。

解释器加载不了动态库，所以 `tests/end_to_end.rs` 在 Miri 下被编译掉。其余套件照跑：

```sh
MIRIFLAGS=-Zmiri-ignore-leaks cargo +nightly miri test -- --test-threads=1
```

`-Zmiri-ignore-leaks` 是给那些把分配器计数放在 `'static` 值里的目标、以及那些建表
的目标用的，因为表缓存按设计是进程级的。两样都不做的目标开着泄漏检查跑：

```sh
cargo +nightly miri test --test checked_raw --test editing --test handles \
    --test hostile --test layout --test primitives --test status --test tuples_arrays \
    --test types --test unwind --test versions -- --test-threads=1
```

## 范围

- **`validate` 校验的是一个值，不是给恶意的生产方做沙箱。** 它拒绝值自己的数字自相
  矛盾的那些：长度超过容量、tag 不是该类型定义的、地址不对齐、小数部分达到或超过
  一秒。它无法知道一个地址背后那块内存实际有多大，所以一个在一字节的块上报告
  `length == capacity == 1_000_000` 的头部通过全部检查，而随后的读取或释放会越过它。
  `load` 是 `unsafe`，原因相同：只加载你已经信任其代码的库。一个必须扛住恶意库的
  协议，必须把额度交给接收侧——它可以读多少字节——并在读之前拿每个值对照它检查。
  本 crate 在四个构造点上收这个额度，
  [`StableStr::from_raw_parts_checked`]、[`StableSlice::from_raw_parts_checked`]、
  [`StableSliceMut::from_raw_parts_checked`] 与
  [`StableBox::from_raw_parts_checked`]；它们只在构造时比对，访问器与其余构造点相信
  值里的数字。
- descriptor 只带一个对象。发布多个服务的库，要么导出多个入口点，要么通过第一个
  对象把其余对象交出来；`plugins/abi_plugin` 走第二条，`tests/end_to_end.rs` 跨动态
  边界断言这一点。
- 宿主提供分配器、日志汇，以及一个查询函数应付其余一切。`plugins/abi_plugin` 按
  标识符请求一个宿主服务，测试观察宿主侧的计数器。
- 泛型方法与关联项在声明处被报出来。一个固定的槽位承载不了它们；诊断会点名能承载
  的形状，而「方法的形状」一节给出了每个实例化一个方法的写法。返回类型里带生命
  周期，以及 `-> Self`，同样在声明处被拒。
- `StableMap` 与 `StableSet` 按键排序，并用二分查找找一个键；[`StableHashMap`] 与
  [`StableHashSet`] 把调用方的哈希留在每个表项旁边并按 `(hash, key)` 排序，所以它们
  用二分查找找键，同时仍然守住本仓的规则：内容相等就占相等的字节，无论表项是按什么
  顺序插进去的。哈希函数是一个参数，所以本 crate 不带 hasher，也不取随机种子：同样
  的表项在每个进程里产出同一个值。表项身份是 `(hash, key)`，所以 map 要用填它的那个
  函数去读，而 [`StableHashMap::validate_with`] 用一个收到的值将要被读取时所用的函数
  去检查它。
- 时间以 UTC 或带数字偏移的 RFC 3339 写出与读入（[`StableSystemTime::to_rfc3339`]、
  [`StableSystemTime::to_rfc3339_with_offset`]、
  [`StableSystemTime::from_rfc3339`]）。换算到本地时区需要那个时区在所涉日期上的
  规则，那些规则来自一个本 crate 不携带的数据库；持有那个数据库的一侧把偏移查出来
  传进来，时刻就按那个时区写出。
- `wipe` 通过 [`Wipe`] trait 提供给纯数据类型，以及能擦除自己元素的容器：字符串、
  向量（包括装着字符串的向量）、map、set、box、数组、带标记的值、写时复制的值与
  错误。实现会覆写内容并让值保持有效，所以之后这个值仍然通过它的分配器释放。让值
  变短的那些操作（`truncate`、`remove`、`clear`、`take`）把让出去的字节留在原地；
  覆写它们是显式的一步。
- 拥有所有权的容器上的 `Debug` 报告形状，不报告内容，因为内容可能是日志不该重复的
  数据。用 `as_slice`/`iter` 去读它们。
- `unwind::guarded` 会跑进程的 panic hook，所以被兜住的 panic 除了作为 [`AbiError`]
  返回之外，也会写进标准错误。一个不能往那里写的宿主装自己的 hook；本 crate 不装，
  因为替换一个进程级的 hook 是宿主的决定。`AbiError::wipe` 覆写一条不该活过这次调用
  的消息。
- 本 crate 不打开动态库，也不维护插件注册表。它提供这样的宿主需要的声明、检查与
  keep-alive 槽位；加载哪些库、什么时候卸载它们，是宿主的决定。
- 本 crate 用 `default-features = false` 在无 `std` 下构建。`std` feature 默认开着，
  门控 `StableOsStr`、`StableOsString`、`StablePath`、`StablePathBuf`、
  `StableSystemTime`、`StableInstant`、`std::error::Error` 的实现，以及
  `stable_object_guarded!`——它的全部工作就是兜住 panic。其余一切，包括
  `stable_object!`、`StableVec`、`StableString`、`StableMap`、`StableSet` 与入口协议，
  两种情况下都可用。分配 crate 以 `heap` 的名字导入，因为本 crate 有一个叫 `alloc`
  的模块——那是 descriptor 携带的分配器句柄。
- 这棵树验证过的最旧工具链是 1.90.0（也用 1.97.1 构建过），这正是 `rust-version`
  写明的。

## 文档

| 文档 | 中文 | English |
| --- | --- | --- |
| 使用与发布提醒 | [`docs/boundary-guidance.zh-CN.md`](docs/boundary-guidance.zh-CN.md) | [`docs/boundary-guidance.md`](docs/boundary-guidance.md) |
| 依赖关系与场景 | [`demo/README.zh-CN.md`](demo/README.zh-CN.md) | [`demo/README.md`](demo/README.md) |
| 本文件 | `README.zh-CN.md` | [`README.md`](README.md) |

## 许可

见 [`LICENSE`](LICENSE)。

仓库是公开的，但条款不是开源授权：XT Engine 之外的使用必须是非营利的，再分发、
修改后再分发、发布到 registry 都不被允许。`Cargo.toml` 用 `license-file` 指向那个
文件，并出于同一个理由写着 `publish = false`。

阅读不受限制：照着这个仓库学到的东西自己写出来的实现，是你自己的作品；留在自己
手上的一份副本也不构成分发。

`repository` 还没写：URL 尚未确定。
