# 参与 Video Perception

感谢你帮助 Agent 以透明、可控的方式理解视频。

- 用户或项目自己的 Provider、Strategy 和 Knowledge Pack 不需要向核心仓库提交 PR；可以独立发布，并用 `video-perception extension validate` 验证。
- 新增标准 capability 或 built-in 实现前，请先提交 design issue，说明输入输出、本地或远程属性、前置条件、失败行为和 streaming 策略。
- 不要提交公司凭证、私有 endpoint、消费端 profile、模型权重或无授权测试视频。
- 修改 capability 时，要同步 registry、执行适配、schema、测试、streaming metadata、文档和 Agent Skill。
- provider 报错不能伪装成空结果成功；`capabilities` 和 `setup` 必须保持只读，不能自动安装软件或读取凭证。
- 贡献 Knowledge Pack 时，不得在许可证不允许的情况下再分发教程、数据集、模型权重或文档。

本地验证请参考英文版 [CONTRIBUTING.md](CONTRIBUTING.md)。架构决策放入 `docs/decisions/`，重要事故复盘放入 `docs/postmortems/`。

提交贡献即表示你同意以 MIT License 发布该贡献。
