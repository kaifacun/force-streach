//#region 스트레칭 동작 안내 문구
pub const TIPS: &[&str] = &[
    "자리에서 일어나 양팔을 머리 위로 쭉 뻗고 10초 버티세요",
    "목을 천천히 좌우로 기울여 각 15초씩 늘려주세요",
    "어깨를 귀까지 올렸다가 툭 떨어뜨리기 — 10회",
    "손바닥을 앞으로 뻗고 반대 손으로 손가락을 몸쪽으로 당기세요 (손목)",
    "양손을 허리에 대고 상체를 뒤로 천천히 젖혀주세요",
    "깍지를 등 뒤에서 끼고 가슴을 활짝 열어주세요",
    "한쪽 팔을 머리 위로 넘겨 옆구리를 늘려주세요 — 좌우 각 15초",
    "벽을 짚고 한 발을 뒤로 빼서 종아리를 늘려주세요",
    "창밖 6m 이상 먼 곳을 20초 이상 바라보세요 (눈 휴식)",
    "제자리 걷기 30초 — 혈액순환을 깨워주세요",
    "턱을 뒤로 당겨 이중턱 만들기 — 거북목 교정 10회",
    "의자 없이 스쿼트 10회!",
];
//#endregion

//#region 앉은 자세 귀띔 (2초 안에 읽히도록 짧게)
pub struct PostureTip {
    pub icon: &'static str,
    pub title: &'static str,
    pub detail: &'static str,
}

pub const POSTURE_TIPS: &[PostureTip] = &[
    PostureTip { icon: "🐢", title: "거북목 주의", detail: "턱을 살짝 당기고, 귀를 어깨선 위로" },
    PostureTip { icon: "🪑", title: "허리 펴기", detail: "엉덩이를 의자 깊숙이, 등을 등받이에" },
    PostureTip { icon: "🤷", title: "어깨 힘 빼기", detail: "올라간 어깨를 툭 내려놓으세요" },
    PostureTip { icon: "🦵", title: "다리 꼬지 않기", detail: "두 발을 바닥에 평평하게" },
    PostureTip { icon: "💻", title: "화면 눈높이", detail: "모니터 윗선이 눈높이에 오도록" },
    PostureTip { icon: "✋", title: "손목 펴기", detail: "손목을 꺾지 말고, 팔꿈치는 90도" },
    PostureTip { icon: "👀", title: "눈 깜빡이기", detail: "화면에서 눈을 떼고 천천히 다섯 번" },
    PostureTip { icon: "🌿", title: "숨 고르기", detail: "가슴을 펴고 깊게 한 번 들이쉬세요" },
];
//#endregion
