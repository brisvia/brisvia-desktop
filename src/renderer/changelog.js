// Per-version, per-language "What's new" notes, baked into the app. Shown ONCE right after an update
// (see maybeShowWhatsNew in app.js), in the language the app is currently set to — never a single-language
// server string. Keep each line short, plain-language, no jargon and no financial claims. On every release,
// add a new top-level version key (e.g. '1.1.5') with the five languages; the modal reads the entry whose key
// equals the running app version. Missing language -> English fallback; missing version -> no modal.
window.CHANGELOG = {
  // Modal chrome (title prefix + button label), per language. The running version is appended to the title.
  _ui: {
    es: { title: 'Novedades de la versión', ok: 'Entendido' },
    en: { title: "What's new in version", ok: 'Got it' },
    pt: { title: 'Novidades da versão', ok: 'Entendi' },
    zh: { title: '版本更新内容', ok: '知道了' },
    ru: { title: 'Что нового в версии', ok: 'Понятно' },
  },
  '1.1.5': {
    es: '• «Tus aportes aceptados» ya no vuelven a cero cuando cambiás la potencia o el minero se reconecta.\n• Estado de conexión más claro: mientras se reconecta solo, ya no aparece un cartel de error.\n• Pantalla de minado más limpia: sacamos información que estaba repetida.\n• Reconexión más confiable y arreglos internos para que el tiempo minado se cuente bien.',
    en: '• “Your accepted work” no longer resets to zero when you change power or the miner reconnects.\n• Clearer connection status: while it reconnects on its own, no scary error message pops up.\n• Cleaner mining screen: we removed information that was repeated.\n• More reliable reconnection and internal fixes so your mining time is counted correctly.',
    pt: '• “Sua contribuição aceita” não zera mais quando você muda a potência ou o minerador reconecta.\n• Status de conexão mais claro: enquanto reconecta sozinho, não aparece mais uma mensagem de erro.\n• Tela de mineração mais limpa: removemos informações que estavam repetidas.\n• Reconexão mais confiável e correções internas para que o tempo minerado seja contado corretamente.',
    zh: '• 更改功率或矿机重新连接时，“你的已接受贡献”不再归零。\n• 连接状态更清晰：自动重连时不再弹出吓人的错误提示。\n• 挖矿界面更简洁：移除了重复显示的信息。\n• 重连更可靠，并修复了内部问题，使挖矿时间正确计算。',
    ru: '• «Ваш принятый вклад» больше не сбрасывается в ноль при смене мощности или переподключении майнера.\n• Понятнее статус подключения: во время автоматического переподключения больше нет пугающего сообщения об ошибке.\n• Более чистый экран майнинга: убрали информацию, которая повторялась.\n• Более надёжное переподключение и внутренние исправления, чтобы время майнинга считалось правильно.',
  },
  '1.1.4': {
    es: '• El minero en modo Solo ahora muestra la velocidad real (antes marcaba 0).\n• En Grupo, «Tu aporte» muestra lo que aportaste en esta sesión; el pago siempre cae en tu Billetera.\n• Al cambiar de tipo de minado ahora te pedimos confirmar antes.\n• La potencia que elegiste se mantiene al cambiar de tipo de minado.',
    en: '• Solo mode now shows your real hashrate (it used to read 0).\n• In a pool, “Your contribution” now shows this session’s accepted work; the payout always lands in your Wallet.\n• Switching mining type now asks you to confirm first.\n• Your chosen power is kept when you change mining type.',
    pt: '• O modo Solo agora mostra a velocidade real (antes marcava 0).\n• No grupo, “Sua contribuição” agora mostra o que você contribuiu nesta sessão; o pagamento sempre cai na sua Carteira.\n• Ao trocar o tipo de mineração, agora pedimos confirmação antes.\n• A potência escolhida é mantida ao trocar o tipo de mineração.',
    zh: '• 单独挖矿模式现在显示真实算力（以前显示 0）。\n• 在矿池中，“你的贡献”现在显示本次会话已接受的工作；收益始终进入你的钱包。\n• 切换挖矿类型时，现在会先请你确认。\n• 切换挖矿类型时会保留你选择的功率。',
    ru: '• Режим соло теперь показывает реальную скорость (раньше было 0).\n• В пуле «Ваш вклад» теперь показывает принятую за сессию работу; выплата всегда приходит в ваш Кошелёк.\n• При смене типа майнинга теперь запрашивается подтверждение.\n• Выбранная мощность сохраняется при смене типа майнинга.',
  },
};
