//! On-screen help messages (message.c msgPost / msgDraw): a parchment box over
//! the player with ENGLISH.ROM text and an announcer line, shown once per the
//! message's "seen" rule, with hints spaced out by a growing delay.

/// One gMsgDescTable entry: priority, seen-rule category, ENGLISH.ROM message
/// (or -1: never shown), which line (-1 = all), and the announcer sound.
#[derive(Clone, Copy, Debug)]
pub struct MsgDesc {
    pub priority: i32,
    pub category: i32,
    pub text: i32,
    pub line: i32,
    pub sound: &'static str,
}

/// gMsgDescTable (0x80124E98), from the decomp.
pub const TABLE: [MsgDesc; 256] = [
    MsgDesc {
        priority: 50,
        category: 3,
        text: 34,
        line: -1,
        sound: "S_USEMAGIC",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 35,
        line: -1,
        sound: "S_USEKEY",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 36,
        line: -1,
        sound: "S_USEKEY2",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 37,
        line: -1,
        sound: "S_MAGICFULL",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 38,
        line: -1,
        sound: "S_KEYFULL",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 68,
        line: -1,
        sound: "S_TRAPSMAKE",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 40,
        line: -1,
        sound: "S_COLLECTPOT",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 41,
        line: -1,
        sound: "S_USEMAGIC2",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 42,
        line: -1,
        sound: "S_SAVEKEYS",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 43,
        line: -1,
        sound: "S_TRANSPORTER",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: -1,
        line: -1,
        sound: "",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 44,
        line: -1,
        sound: "S_SAMEEXIT",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: -1,
        line: -1,
        sound: "",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: -1,
        line: -1,
        sound: "",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 47,
        line: -1,
        sound: "S_SHOOTINGMAGIC",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 141,
        line: -1,
        sound: "S_MEATGIVES",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 142,
        line: -1,
        sound: "S_FRUITGIVES",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 91,
        line: -1,
        sound: "S_COLLECTGOLD",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 48,
        line: -1,
        sound: "S_DONTWASTE",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 49,
        line: -1,
        sound: "S_YOUNEED",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 50,
        line: -1,
        sound: "S_MULTIPLEHITS",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 51,
        line: -1,
        sound: "S_AVOID",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 52,
        line: -1,
        sound: "S_DESTROY",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 53,
        line: -1,
        sound: "S_SILVER",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: -1,
        line: -1,
        sound: "",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: -1,
        line: -1,
        sound: "",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: -1,
        line: -1,
        sound: "",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 54,
        line: -1,
        sound: "S_SOMEBARRELS",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 92,
        line: -1,
        sound: "S_POISONEDFOOD",
    },
    MsgDesc {
        priority: 50,
        category: 0,
        text: 55,
        line: -1,
        sound: "S_FOLLOW",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: -1,
        line: -1,
        sound: "S_XSTRENGTH",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: -1,
        line: -1,
        sound: "S_XARMORN",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 95,
        line: -1,
        sound: "S_XSPEED",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 96,
        line: -1,
        sound: "S_XMAGIC",
    },
    MsgDesc {
        priority: 50,
        category: 0,
        text: 97,
        line: -1,
        sound: "S_GAINEDLEVEL",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 99,
        line: -1,
        sound: "S_INVULVOX",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 98,
        line: -1,
        sound: "S_INVISVOX",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 100,
        line: -1,
        sound: "S_3WAYSHOTVOX",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 101,
        line: -1,
        sound: "S_REFLECTVOX",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 102,
        line: -1,
        sound: "S_XRAYVOX",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 103,
        line: -1,
        sound: "S_FIREAMVOX",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 104,
        line: -1,
        sound: "S_LGHTNGAMVOX",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 105,
        line: -1,
        sound: "S_LIGHTAMVOX",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 106,
        line: -1,
        sound: "S_ACIDAMVOX",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 56,
        line: -1,
        sound: "S_SHOOTRED",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 57,
        line: -1,
        sound: "S_SHOOTGREEN",
    },
    MsgDesc {
        priority: 50,
        category: 2,
        text: 58,
        line: -1,
        sound: "",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 107,
        line: -1,
        sound: "S_5WAYSHOTVOX",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 108,
        line: -1,
        sound: "S_SUPERVOX",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 109,
        line: -1,
        sound: "S_ANTIDEATHVOX",
    },
    MsgDesc {
        priority: 40,
        category: 0,
        text: 110,
        line: -1,
        sound: "S_NOWIT",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 111,
        line: -1,
        sound: "S_STOPPEDVOX",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 112,
        line: -1,
        sound: "S_REFLECTSHVOX",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 113,
        line: -1,
        sound: "S_LEVVOX",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 99,
        line: -1,
        sound: "S_INVULVOX",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: -1,
        line: -1,
        sound: "",
    },
    MsgDesc {
        priority: 50,
        category: 2,
        text: -1,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 60,
        category: 1,
        text: 25,
        line: 1,
        sound: "S_FIREARC",
    },
    MsgDesc {
        priority: 70,
        category: 1,
        text: 25,
        line: 2,
        sound: "S_PLASMATRAIL",
    },
    MsgDesc {
        priority: 50,
        category: 2,
        text: -1,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 60,
        category: 1,
        text: 26,
        line: 1,
        sound: "S_MULTIBLADE",
    },
    MsgDesc {
        priority: 70,
        category: 1,
        text: 26,
        line: 2,
        sound: "S_SKYLANCE",
    },
    MsgDesc {
        priority: 50,
        category: 2,
        text: -1,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 60,
        category: 1,
        text: 27,
        line: 1,
        sound: "S_ROCKSHOWER",
    },
    MsgDesc {
        priority: 70,
        category: 1,
        text: 27,
        line: 2,
        sound: "S_DEMONSKULL",
    },
    MsgDesc {
        priority: 50,
        category: 2,
        text: -1,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 60,
        category: 1,
        text: 28,
        line: 1,
        sound: "S_DOUBLEBOW",
    },
    MsgDesc {
        priority: 70,
        category: 1,
        text: 28,
        line: 2,
        sound: "S_BFG",
    },
    MsgDesc {
        priority: 50,
        category: 2,
        text: -1,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 60,
        category: 1,
        text: 29,
        line: 1,
        sound: "S_TURB_DWF",
    },
    MsgDesc {
        priority: 70,
        category: 1,
        text: 29,
        line: 2,
        sound: "S_TURC_DWF",
    },
    MsgDesc {
        priority: 50,
        category: 2,
        text: -1,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 60,
        category: 1,
        text: 30,
        line: 1,
        sound: "S_TURB_KNI",
    },
    MsgDesc {
        priority: 70,
        category: 1,
        text: 30,
        line: 2,
        sound: "S_TURC_KNI",
    },
    MsgDesc {
        priority: 50,
        category: 2,
        text: -1,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 60,
        category: 1,
        text: 31,
        line: 1,
        sound: "S_TURB_SOR",
    },
    MsgDesc {
        priority: 70,
        category: 1,
        text: 31,
        line: 2,
        sound: "S_TURC_SOR",
    },
    MsgDesc {
        priority: 50,
        category: 2,
        text: -1,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 60,
        category: 1,
        text: 32,
        line: 1,
        sound: "S_TURB_JES",
    },
    MsgDesc {
        priority: 70,
        category: 1,
        text: 32,
        line: 2,
        sound: "S_TURC_JES",
    },
    MsgDesc {
        priority: 90,
        category: 3,
        text: -1,
        line: -1,
        sound: "",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 114,
        line: -1,
        sound: "S_FIREBRVOX",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 115,
        line: -1,
        sound: "S_ACIDBRVOX",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 116,
        line: -1,
        sound: "S_LGHTNGBRVOX",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 117,
        line: -1,
        sound: "S_PHOENIXVOX",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 118,
        line: -1,
        sound: "S_LIGHTBRVOX",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 119,
        line: -1,
        sound: "S_HAMMERVOX",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 120,
        line: -1,
        sound: "S_RAPIDFIREVOX",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 121,
        line: -1,
        sound: "S_GROWTHVOX",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 122,
        line: -1,
        sound: "S_SHRINKVOX",
    },
    MsgDesc {
        priority: 50,
        category: 0,
        text: 59,
        line: -1,
        sound: "S_ALREADYRUNE",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 123,
        line: -1,
        sound: "S_FIREWALLSHVOX",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 124,
        line: -1,
        sound: "S_LGHTNGSHVOX",
    },
    MsgDesc {
        priority: 50,
        category: 0,
        text: 125,
        line: -1,
        sound: "S_POJOVOX",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 61,
        line: -1,
        sound: "S_THROWMAGIC",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 62,
        line: -1,
        sound: "S_SHIELDMAGIC",
    },
    MsgDesc {
        priority: 90,
        category: 0,
        text: -1,
        line: -1,
        sound: "",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 64,
        line: -1,
        sound: "",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 126,
        line: -1,
        sound: "S_MASKVOX",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 127,
        line: -1,
        sound: "S_HORNSVOX",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 128,
        line: -1,
        sound: "S_GAUNTLETVOX",
    },
    MsgDesc {
        priority: 50,
        category: 0,
        text: 24,
        line: -1,
        sound: "S_GAINEDLEVEL",
    },
    MsgDesc {
        priority: 70,
        category: 2,
        text: 33,
        line: 0,
        sound: "S_PINBALL",
    },
    MsgDesc {
        priority: 70,
        category: 2,
        text: 33,
        line: 1,
        sound: "S_SPINKICK",
    },
    MsgDesc {
        priority: 70,
        category: 2,
        text: 33,
        line: 2,
        sound: "S_DOPPEL",
    },
    MsgDesc {
        priority: 70,
        category: 2,
        text: 33,
        line: 3,
        sound: "S_AERIAL",
    },
    MsgDesc {
        priority: 70,
        category: 2,
        text: 33,
        line: 4,
        sound: "S_SCHARGE",
    },
    MsgDesc {
        priority: 70,
        category: 2,
        text: 33,
        line: 5,
        sound: "S_DBOMBER",
    },
    MsgDesc {
        priority: 70,
        category: 2,
        text: 33,
        line: 6,
        sound: "S_BIGFOOT",
    },
    MsgDesc {
        priority: 70,
        category: 2,
        text: 33,
        line: 7,
        sound: "S_CANNONBALL",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 65,
        line: -1,
        sound: "S_USETURBO",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 66,
        line: -1,
        sound: "S_USECOMBO",
    },
    MsgDesc {
        priority: 50,
        category: 1,
        text: -1,
        line: -1,
        sound: "",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 129,
        line: -1,
        sound: "S_TURBOBOOST",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 130,
        line: -1,
        sound: "S_SCIMITARVOX",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 131,
        line: -1,
        sound: "S_ICEAXEVOX",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 132,
        line: -1,
        sound: "S_LAMPVOX",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 133,
        line: -1,
        sound: "S_BELLOWSVOX",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 134,
        line: -1,
        sound: "S_SAVIORVOX",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 135,
        line: -1,
        sound: "S_SAVIORVOX",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 136,
        line: -1,
        sound: "S_BOOKVOX",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 137,
        line: -1,
        sound: "S_SAVIORVOX",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 138,
        line: -1,
        sound: "S_PARCHVOX",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 139,
        line: -1,
        sound: "S_LANTERNVOX",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 140,
        line: -1,
        sound: "S_JAVELINVOX",
    },
    MsgDesc {
        priority: 50,
        category: 2,
        text: 69,
        line: -1,
        sound: "S_LEARNBLOCK",
    },
    MsgDesc {
        priority: 50,
        category: 1,
        text: 70,
        line: -1,
        sound: "S_ALLPLATFRM",
    },
    MsgDesc {
        priority: 50,
        category: 1,
        text: 71,
        line: -1,
        sound: "S_TRIGGERVOX",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 72,
        line: -1,
        sound: "S_DEATHDRAINXP",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 73,
        line: -1,
        sound: "S_DIESAFTERXP",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 74,
        line: -1,
        sound: "S_DEATHDRAINS",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 75,
        line: -1,
        sound: "S_DIESAFTER",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 143,
        line: -1,
        sound: "S_GASMASK",
    },
    MsgDesc {
        priority: 50,
        category: 2,
        text: 76,
        line: -1,
        sound: "S_HEALTHFULL",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 77,
        line: -1,
        sound: "S_GENSCARRY",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 78,
        line: -1,
        sound: "S_EXPDSTITMS",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 79,
        line: -1,
        sound: "S_GASFOODBAD",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 80,
        line: -1,
        sound: "S_CHESTSEXPL",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 81,
        line: -1,
        sound: "S_DEFGRG4GLD",
    },
    MsgDesc {
        priority: 50,
        category: 2,
        text: 82,
        line: -1,
        sound: "S_MAGJNK2SILV",
    },
    MsgDesc {
        priority: 50,
        category: 2,
        text: 83,
        line: -1,
        sound: "S_MAGJNK2GLD",
    },
    MsgDesc {
        priority: 50,
        category: 2,
        text: 84,
        line: -1,
        sound: "S_MAGSTOPTRP",
    },
    MsgDesc {
        priority: 50,
        category: 2,
        text: 85,
        line: -1,
        sound: "S_MAGDSTTRP",
    },
    MsgDesc {
        priority: 50,
        category: 2,
        text: 86,
        line: -1,
        sound: "S_MAGCLFRUIT",
    },
    MsgDesc {
        priority: 50,
        category: 2,
        text: 87,
        line: -1,
        sound: "S_MAGCLMEAT",
    },
    MsgDesc {
        priority: 50,
        category: 2,
        text: 88,
        line: -1,
        sound: "S_MAGSHOWWALLS",
    },
    MsgDesc {
        priority: 50,
        category: 2,
        text: 89,
        line: -1,
        sound: "S_MAGDSTWLLS",
    },
    MsgDesc {
        priority: 50,
        category: 2,
        text: 90,
        line: -1,
        sound: "S_MAGICHEAL",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 202,
        line: -1,
        sound: "S_PICKUPCRYST",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 203,
        line: -1,
        sound: "S_PICKUPCRYST",
    },
    MsgDesc {
        priority: 50,
        category: 3,
        text: 204,
        line: -1,
        sound: "S_PICKUPCRYST",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
    MsgDesc {
        priority: 0,
        category: 0,
        text: 0,
        line: 0,
        sound: "",
    },
];

/// The hint delay ladder in ticks (message config 0x80124E58).
const DELAYS: [i32; 5] = [0, 120, 240, 420, 600];

/// A message on screen.
#[derive(Clone, Debug)]
pub struct Shown {
    pub index: usize,
    /// The player it's about (its box sits over them), if any.
    pub player: Option<usize>,
    pub lines: Vec<String>,
    /// Ticks left.
    pub ticks: i32,
}

#[derive(Clone, Debug, Default)]
pub struct Messages {
    /// ENGLISH.ROM messages, by index.
    pub text: Vec<Vec<String>>,
    pub shown: Option<Shown>,
    /// Each player's save.help_disp: per message, SEEN_NOW (since the hero
    /// was loaded) | SEEN_EVER once shown (empty = none seen).
    help_disp: [Vec<u8>; 4],
    delay: i32,
    step: usize,
    /// Announcer lines to play, drained by the game.
    pub voice: Vec<String>,
}

impl Messages {
    pub fn new(text: Vec<Vec<String>>) -> Messages {
        Messages {
            text,
            ..Default::default()
        }
    }

    /// Player `p`'s help_disp, for its save.
    pub fn help_disp(&self, p: usize) -> [u8; 256] {
        let mut out = [0; 256];
        if let Some(hd) = self.help_disp.get(p) {
            for (o, b) in out.iter_mut().zip(hd) {
                *o = *b;
            }
        }
        out
    }

    /// Player `p` brings the messages its save has seen.
    pub fn set_help_disp(&mut self, p: usize, hd: &[u8; 256]) {
        if let Some(d) = self.help_disp.get_mut(p) {
            *d = hd.to_vec();
        }
    }

    /// msgPost: show message `index` about `player` (None for the party), with
    /// `%s`/`%d` filled from `args`. `active` lists the players in play, for the
    /// seen rules. Returns whether it was shown.
    pub fn post(
        &mut self,
        index: usize,
        player: Option<usize>,
        active: &[usize],
        args: &[String],
    ) -> bool {
        let Some(d) = TABLE.get(index) else {
            return false;
        };
        if d.text < 0 {
            return false;
        }
        let pl = player.map(|p| p.min(3));
        let flags = |p: usize| self.help_disp[p].get(index).copied().unwrap_or(0);
        // msgWorldFlags: category 1 checks this session's bit, 2 and 3 any.
        let ok = match d.category {
            0 => true,
            1 => !active.iter().any(|&p| flags(p) & 0xF != 0),
            2 => match pl {
                Some(p) => flags(p) == 0,
                None => !active.iter().any(|&p| flags(p) != 0),
            },
            _ => !active.iter().all(|&p| flags(p) != 0),
        };
        if !ok {
            return false;
        }
        if let Some(s) = &self.shown {
            if TABLE[s.index].priority >= d.priority {
                return false;
            }
        }
        // Hints wait out the delay; announcements don't.
        if (index <= 0x1C || [0x2C, 0x2D, 0x37, 0x50].contains(&index)) && self.delay > 0 {
            return false;
        }
        let Some(all) = self.text.get(d.text as usize) else {
            return false;
        };
        let mut lines: Vec<String> = if d.line >= 0 {
            all.get(d.line as usize).cloned().into_iter().collect()
        } else {
            all.clone()
        };
        // Lines may hold several rows; fill %s / %d in order.
        let mut args = args.iter();
        for l in &mut lines {
            while let Some(i) = l.find("%s").or_else(|| l.find("%d")) {
                let a = args.next().cloned().unwrap_or_default();
                l.replace_range(i..i + 2, &a);
            }
        }
        let lines: Vec<String> = lines
            .iter()
            .flat_map(|l| l.split('\n').map(str::to_string).collect::<Vec<_>>())
            .filter(|l| !l.is_empty())
            .collect();
        let n = if d.line >= 0 { 1 } else { all.len().max(1) };
        for &p in active {
            if d.category == 0 || (d.category == 2 && pl.is_some_and(|pl| p != pl)) {
                continue;
            }
            let hd = &mut self.help_disp[p.min(3)];
            if hd.len() < 256 {
                hd.resize(256, 0);
            }
            hd[index] |= crate::save::SEEN_NOW | crate::save::SEEN_EVER;
        }
        self.shown = Some(Shown {
            index,
            player,
            lines,
            ticks: if index == 0x22 {
                150
            } else {
                n as i32 * 60 + 30
            },
        });
        if !d.sound.is_empty() {
            self.voice.push(format!("VOICE:{}", d.sound));
        }
        self.delay = DELAYS[self.step.min(DELAYS.len() - 1)];
        self.step += 1;
        true
    }

    /// One tick (frozen while the game is).
    pub fn tick(&mut self) {
        self.delay = (self.delay - 1).max(0);
        if let Some(s) = &mut self.shown {
            s.ticks -= 1;
            if s.ticks <= 0 {
                self.shown = None;
            }
        }
    }
}

/// Text colours per player, and for messages about no one.
pub const PLAYER_TEXT: [u32; 4] = [0x1F1F00, 0x00001F, 0x1F0000, 0x001F00];
pub const NEUTRAL_TEXT: u32 = 0x160C03;

#[cfg(test)]
mod tests {
    use super::*;

    fn rom() -> Vec<Vec<String>> {
        let mut t = vec![Vec::new(); 260];
        t[141] = vec!["MEAT GIVES MORE HEALTH".into()];
        t[142] = vec!["FRUIT GIVES SOME HEALTH".into()];
        t[97] = vec!["LEVEL %d\nEXPERIENCE".into()];
        t[91] = vec!["COLLECT GOLD".into(), "TO BUY POWERUPS".into()];
        t
    }

    /// help_disp goes into the save: after a load only "seen at all" is
    /// left, which still holds back categories 2 and 3; category 1 is once
    /// per session.
    #[test]
    fn seen_messages_survive_a_save() {
        use crate::save::{HeroSave, SEEN_EVER, SEEN_NOW};
        let mut r = rom();
        r[25] = vec!["ONCE A SESSION".into()];
        r[58] = vec!["THIS PLAYER".into()];
        let mut m = Messages::new(r.clone());
        for i in [15, 46, 57] {
            assert!(m.post(i, Some(0), &[0], &[]), "{i}");
            m.shown = None;
        }
        let hd = m.help_disp(0);
        assert_eq!(hd[15], SEEN_NOW | SEEN_EVER);
        assert_eq!(m.help_disp(1)[15], 0);
        let mut h = HeroSave::new("X", 0, 0);
        h.help_disp = hd;
        let h = h.loaded();
        let mut m = Messages::new(r);
        m.set_help_disp(0, &h.help_disp);
        assert!(!m.post(15, Some(0), &[0], &[]), "category 3: seen before");
        assert!(!m.post(46, Some(0), &[0], &[]), "category 2: seen before");
        assert!(m.post(57, Some(0), &[0], &[]), "category 1: a new session");
        m.shown = None;
        // A second hero who has not seen it brings a category 3 hint back.
        assert!(m.post(15, Some(0), &[0, 1], &[]));
    }

    #[test]
    fn hints_show_once_and_wait_their_turn() {
        let mut m = Messages::new(rom());
        assert!(m.post(15, Some(0), &[0], &[]));
        assert_eq!(m.voice, vec!["VOICE:S_MEATGIVES".to_string()]);
        for _ in 0..200 {
            m.tick();
        }
        // Seen by every player: not again.
        assert!(!m.post(15, Some(0), &[0], &[]));
        // The delays run 0, 120, 240...: the second hint may follow at once, the
        // third waits 120 ticks.
        let mut m = Messages::new(rom());
        assert!(m.post(15, Some(0), &[0], &[]));
        m.shown = None;
        assert!(m.post(16, Some(0), &[0], &[]));
        m.shown = None;
        assert!(!m.post(17, Some(0), &[0], &[]));
    }

    #[test]
    fn level_up_fills_in_the_level() {
        let mut m = Messages::new(rom());
        assert!(m.post(0x22, Some(1), &[0, 1], &["3".into()]));
        let s = m.shown.unwrap();
        assert_eq!(
            s.lines,
            vec!["LEVEL 3".to_string(), "EXPERIENCE".to_string()]
        );
        assert_eq!(s.ticks, 150);
    }
}
