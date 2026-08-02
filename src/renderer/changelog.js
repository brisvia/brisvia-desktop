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
    es: '• Nuevo: elegí a qué grupo (pool) minar. El oficial de Brisvia (0% de comisión) o tu propio grupo. Siempre ves a dónde va tu trabajo, y podés probar si el grupo está funcionando (verde) o no (rojo). El pago siempre cae en tu Billetera.\n• «Tus aportes aceptados» ya no vuelven a cero cuando cambiás la potencia o el minero se reconecta.\n• Estado de conexión más claro: mientras se reconecta solo, ya no aparece un cartel de error.\n• Pantalla de minado más limpia y reconexión más confiable.',
    en: '• New: choose which pool to mine to. The official Brisvia pool (0% fee) or your own pool. You always see where your work goes, and you can check whether a pool is running (green) or not (red). The payout always lands in your Wallet.\n• “Your accepted work” no longer resets to zero when you change power or the miner reconnects.\n• Clearer connection status: while it reconnects on its own, no scary error message pops up.\n• Cleaner mining screen and more reliable reconnection.',
    pt: '• Novo: escolha para qual pool minerar. A pool oficial da Brisvia (0% de comissão) ou a sua própria pool. Você sempre vê para onde vai o seu trabalho e pode testar se a pool está funcionando (verde) ou não (vermelho). O pagamento sempre cai na sua Carteira.\n• “Sua contribuição aceita” não zera mais quando você muda a potência ou o minerador reconecta.\n• Status de conexão mais claro: enquanto reconecta sozinho, não aparece mais uma mensagem de erro.\n• Tela de mineração mais limpa e reconexão mais confiável.',
    zh: '• 新功能：选择要挖到哪个矿池。Brisvia 官方矿池（0% 手续费）或你自己的矿池。你始终能看到你的工作去向，并可检测矿池是否在运行（绿色）或未运行（红色）。收益始终进入你的钱包。\n• 更改功率或矿机重新连接时，“你的已接受贡献”不再归零。\n• 连接状态更清晰：自动重连时不再弹出吓人的错误提示。\n• 挖矿界面更简洁，重连更可靠。',
    ru: '• Новое: выбирайте, в какой пул майнить. Официальный пул Brisvia (0% комиссии) или ваш собственный пул. Вы всегда видите, куда идёт ваша работа, и можете проверить, работает пул (зелёный) или нет (красный). Выплата всегда приходит в ваш Кошелёк.\n• «Ваш принятый вклад» больше не сбрасывается в ноль при смене мощности или переподключении майнера.\n• Понятнее статус подключения: во время автоматического переподключения больше нет пугающего сообщения об ошибке.\n• Более чистый экран майнинга и более надёжное переподключение.',
  },
  '1.1.4': {
    es: '• El minero en modo Solo ahora muestra la velocidad real (antes marcaba 0).\n• En Grupo, «Tu aporte» muestra lo que aportaste en esta sesión; el pago siempre cae en tu Billetera.\n• Al cambiar de tipo de minado ahora te pedimos confirmar antes.\n• La potencia que elegiste se mantiene al cambiar de tipo de minado.',
    en: '• Solo mode now shows your real hashrate (it used to read 0).\n• In a pool, “Your contribution” now shows this session’s accepted work; the payout always lands in your Wallet.\n• Switching mining type now asks you to confirm first.\n• Your chosen power is kept when you change mining type.',
    pt: '• O modo Solo agora mostra a velocidade real (antes marcava 0).\n• No grupo, “Sua contribuição” agora mostra o que você contribuiu nesta sessão; o pagamento sempre cai na sua Carteira.\n• Ao trocar o tipo de mineração, agora pedimos confirmação antes.\n• A potência escolhida é mantida ao trocar o tipo de mineração.',
    zh: '• 单独挖矿模式现在显示真实算力（以前显示 0）。\n• 在矿池中，“你的贡献”现在显示本次会话已接受的工作；收益始终进入你的钱包。\n• 切换挖矿类型时，现在会先请你确认。\n• 切换挖矿类型时会保留你选择的功率。',
    ru: '• Режим соло теперь показывает реальную скорость (раньше было 0).\n• В пуле «Ваш вклад» теперь показывает принятую за сессию работу; выплата всегда приходит в ваш Кошелёк.\n• При смене типа майнинга теперь запрашивается подтверждение.\n• Выбранная мощность сохраняется при смене типа майнинга.',
  },
};
