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
    es: '• Ahora vos elegís cómo minar: solo (con tu propia compu) o en un grupo. Para usar un grupo, escribí su dirección ahí mismo en la pantalla de Minar. Siempre ves a dónde va tu trabajo y podés probar si el grupo responde (verde) o no (rojo). El pago siempre cae en tu Billetera.\n• Al enviar, ahora podés elegir de qué dirección de tu billetera salen los fondos (o de varias), viendo cuánto hay en cada una.\n• Cambiá el idioma con las banderitas de arriba a la derecha.\n• El detalle de cada movimiento se ve más ancho, y tocando el código de la transacción vas directo al explorador oficial.\n• «Tus aportes aceptados» ya no vuelven a cero al cambiar la potencia o cuando el minero se reconecta.',
    en: '• You now choose how to mine: solo (with your own computer) or in a pool. To use a pool, type its address right on the Mine screen. You always see where your work goes and can check whether the pool responds (green) or not (red). The payout always lands in your Wallet.\n• When sending, you can now choose which of your wallet addresses the funds come from (one or several), seeing how much each holds.\n• Switch language with the flags at the top right.\n• Each transaction’s detail is wider, and tapping the transaction code takes you straight to the official explorer.\n• “Your accepted work” no longer resets to zero when you change power or the miner reconnects.',
    pt: '• Agora você escolhe como minerar: sozinho (com o seu computador) ou em um grupo (pool). Para usar um grupo, digite o endereço dele na própria tela de Mineração. Você sempre vê para onde vai o seu trabalho e pode testar se o grupo responde (verde) ou não (vermelho). O pagamento sempre cai na sua Carteira.\n• Ao enviar, agora você pode escolher de qual endereço da sua carteira saem os fundos (ou de vários), vendo quanto há em cada um.\n• Troque o idioma nas bandeiras no canto superior direito.\n• O detalhe de cada movimentação é mais largo, e tocando no código da transação você vai direto ao explorador oficial.\n• “Sua contribuição aceita” não zera mais ao mudar a potência ou quando o minerador reconecta.',
    zh: '• 现在由你选择如何挖矿：单独挖（用你自己的电脑）或加入矿池。要使用矿池，直接在挖矿界面输入它的地址。你始终能看到你的工作去向，并可检测矿池是否响应（绿色）或未响应（红色）。收益始终进入你的钱包。\n• 发送时，你现在可以选择资金从钱包的哪个地址（或多个地址）支出，并查看每个地址的余额。\n• 用右上角的国旗切换语言。\n• 每笔交易的详情更宽，点击交易代码可直接前往官方浏览器。\n• 更改功率或矿机重新连接时，“你的已接受贡献”不再归零。',
    ru: '• Теперь вы выбираете, как майнить: соло (на своём компьютере) или в пуле. Чтобы использовать пул, впишите его адрес прямо на экране майнинга. Вы всегда видите, куда идёт ваша работа, и можете проверить, отвечает пул (зелёный) или нет (красный). Выплата всегда приходит в ваш Кошелёк.\n• При отправке теперь можно выбрать, с какого адреса вашего кошелька списываются средства (или с нескольких), видя баланс каждого.\n• Переключайте язык флажками в правом верхнем углу.\n• Детали каждой операции шире, а по нажатию на код транзакции вы попадаете прямо в официальный обозреватель.\n• «Ваш принятый вклад» больше не сбрасывается в ноль при смене мощности или переподключении майнера.',
  },
  '1.1.4': {
    es: '• El minero en modo Solo ahora muestra la velocidad real (antes marcaba 0).\n• En Grupo, «Tu aporte» muestra lo que aportaste en esta sesión; el pago siempre cae en tu Billetera.\n• Al cambiar de tipo de minado ahora te pedimos confirmar antes.\n• La potencia que elegiste se mantiene al cambiar de tipo de minado.',
    en: '• Solo mode now shows your real hashrate (it used to read 0).\n• In a pool, “Your contribution” now shows this session’s accepted work; the payout always lands in your Wallet.\n• Switching mining type now asks you to confirm first.\n• Your chosen power is kept when you change mining type.',
    pt: '• O modo Solo agora mostra a velocidade real (antes marcava 0).\n• No grupo, “Sua contribuição” agora mostra o que você contribuiu nesta sessão; o pagamento sempre cai na sua Carteira.\n• Ao trocar o tipo de mineração, agora pedimos confirmação antes.\n• A potência escolhida é mantida ao trocar o tipo de mineração.',
    zh: '• 单独挖矿模式现在显示真实算力（以前显示 0）。\n• 在矿池中，“你的贡献”现在显示本次会话已接受的工作；收益始终进入你的钱包。\n• 切换挖矿类型时，现在会先请你确认。\n• 切换挖矿类型时会保留你选择的功率。',
    ru: '• Режим соло теперь показывает реальную скорость (раньше было 0).\n• В пуле «Ваш вклад» теперь показывает принятую за сессию работу; выплата всегда приходит в ваш Кошелёк.\n• При смене типа майнинга теперь запрашивается подтверждение.\n• Выбранная мощность сохраняется при смене типа майнинга.',
  },
};
