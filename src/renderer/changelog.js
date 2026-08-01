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
  '1.1.4': {
    es: '• El minero en modo Solo ahora muestra la velocidad real (antes marcaba 0).\n• En Grupo, «Tu aporte» muestra lo que aportaste en esta sesión; el pago siempre cae en tu Billetera.\n• Al cambiar de tipo de minado ahora te pedimos confirmar antes.\n• La potencia que elegiste se mantiene al cambiar de tipo de minado.',
    en: '• Solo mode now shows your real hashrate (it used to read 0).\n• In a pool, “Your contribution” now shows this session’s accepted work; the payout always lands in your Wallet.\n• Switching mining type now asks you to confirm first.\n• Your chosen power is kept when you change mining type.',
    pt: '• O modo Solo agora mostra a velocidade real (antes marcava 0).\n• No grupo, “Sua contribuição” agora mostra o que você contribuiu nesta sessão; o pagamento sempre cai na sua Carteira.\n• Ao trocar o tipo de mineração, agora pedimos confirmação antes.\n• A potência escolhida é mantida ao trocar o tipo de mineração.',
    zh: '• 单独挖矿模式现在显示真实算力（以前显示 0）。\n• 在矿池中，“你的贡献”现在显示本次会话已接受的工作；收益始终进入你的钱包。\n• 切换挖矿类型时，现在会先请你确认。\n• 切换挖矿类型时会保留你选择的功率。',
    ru: '• Режим соло теперь показывает реальную скорость (раньше было 0).\n• В пуле «Ваш вклад» теперь показывает принятую за сессию работу; выплата всегда приходит в ваш Кошелёк.\n• При смене типа майнинга теперь запрашивается подтверждение.\n• Выбранная мощность сохраняется при смене типа майнинга.',
  },
};
