fn main() {
    use std::{env, fs, path::PathBuf};

    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=LOG");
    println!("cargo:rerun-if-env-changed=BASE_ADDRESS");
    println!("cargo:rerun-if-env-changed=CHAPTER");
    println!("cargo:rerun-if-env-changed=CC");
    println!("cargo:rerun-if-env-changed=AR");
    println!("cargo:rerun-if-env-changed=TG_ENABLE_DOOM_C");
    println!("cargo:rerun-if-env-changed=TG_DOOM_FULL");
    println!("cargo:rerun-if-changed=src/bin/doomgeneric/doomgeneric_tg.c");
    println!("cargo:rerun-if-changed=src/bin/doomgeneric");
    println!("cargo:rustc-check-cfg=cfg(tg_doom_c)");
    println!("cargo:rustc-check-cfg=cfg(tg_doom_full)");

    if let Ok(chapter) = env::var("CHAPTER") {
        println!("cargo:rustc-env=CHAPTER={chapter}");
    }

    try_build_doom_c();

    if let Some(base) = env::var("BASE_ADDRESS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
    {
        let text = format!(
            "\
OUTPUT_ARCH(riscv)
ENTRY(_start)
SECTIONS {{
    . = {base};
    .text : {{
        *(.text.entry)
        *(.text .text.*)
    }}
    .rodata : {{
        *(.rodata .rodata.*)
        *(.srodata .srodata.*)
    }}
    .data : {{
        *(.data .data.*)
        *(.sdata .sdata.*)
    }}
    .bss : {{
        *(.bss .bss.*)
        *(.sbss .sbss.*)
    }}
}}"
        );
        let ld = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("linker.ld");
        fs::write(&ld, text).unwrap();
        println!("cargo:rustc-link-arg=-T{}", ld.display());
    }
}

fn try_build_doom_c() {
    use std::{env, process::Command};

    let target = env::var("TARGET").unwrap_or_default();
    if !target.starts_with("riscv64") {
        return;
    }

    let enable = env::var("TG_ENABLE_DOOM_C").ok().map(|s| s == "1").unwrap_or(false);
    if !enable {
        return;
    }

    let full = env::var("TG_DOOM_FULL").ok().map(|s| s == "1").unwrap_or(false);

    let compiler = if let Ok(cc) = env::var("CC") {
        if cc.trim().is_empty() {
            None
        } else {
            Some(cc)
        }
    } else if command_exists("riscv64-linux-gnu-gcc") {
        Some("riscv64-linux-gnu-gcc".to_string())
    } else if command_exists("riscv64-unknown-elf-gcc") {
        Some("riscv64-unknown-elf-gcc".to_string())
    } else {
        None
    };

    let Some(compiler) = compiler else {
        println!(
            "cargo:warning=skip doom C build: no cross C compiler found (set CC or install riscv64-unknown-elf-gcc)"
        );
        return;
    };

    let mut build = cc::Build::new();
    build.no_default_flags(true);
    build.compiler(compiler);
    build.target(&target);
    build.include("src/bin/doomgeneric");
    build.flag("-march=rv64gc");
    build.flag("-mabi=lp64d");
    build.flag("-O2");
    build.flag("-ffreestanding");
    build.flag("-fno-builtin");
    build.flag("-fno-stack-protector");
    build.warnings(false);

    if full {
        build.define("TG_DOOM_FULL", None);
        for file in DOOM_GENERIC_FULL_SOURCES {
            build.file(format!("src/bin/doomgeneric/{file}"));
        }
    } else {
        build.file("src/bin/doomgeneric/doomgeneric_tg.c");
    }

    build.compile("doomtg");
    println!("cargo:rustc-cfg=tg_doom_c");
    if full {
        println!("cargo:rustc-cfg=tg_doom_full");
    }

    fn command_exists(cmd: &str) -> bool {
        Command::new("sh")
            .args(["-lc", &format!("command -v {cmd} >/dev/null 2>&1")])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
}

const DOOM_GENERIC_FULL_SOURCES: &[&str] = &[
    "dummy.c",
    "am_map.c",
    "doomdef.c",
    "doomstat.c",
    "dstrings.c",
    "d_event.c",
    "d_items.c",
    "d_iwad.c",
    "d_loop.c",
    "d_main.c",
    "d_mode.c",
    "d_net.c",
    "f_finale.c",
    "f_wipe.c",
    "g_game.c",
    "hu_lib.c",
    "hu_stuff.c",
    "info.c",
    "i_cdmus.c",
    "i_endoom.c",
    "i_joystick.c",
    "i_scale.c",
    "i_sound.c",
    "i_system.c",
    "i_timer.c",
    "memio.c",
    "m_argv.c",
    "m_bbox.c",
    "m_cheat.c",
    "m_config.c",
    "m_controls.c",
    "m_fixed.c",
    "m_menu.c",
    "m_misc.c",
    "m_random.c",
    "p_ceilng.c",
    "p_doors.c",
    "p_enemy.c",
    "p_floor.c",
    "p_inter.c",
    "p_lights.c",
    "p_map.c",
    "p_maputl.c",
    "p_mobj.c",
    "p_plats.c",
    "p_pspr.c",
    "p_saveg.c",
    "p_setup.c",
    "p_sight.c",
    "p_spec.c",
    "p_switch.c",
    "p_telept.c",
    "p_tick.c",
    "p_user.c",
    "r_bsp.c",
    "r_data.c",
    "r_draw.c",
    "r_main.c",
    "r_plane.c",
    "r_segs.c",
    "r_sky.c",
    "r_things.c",
    "sha1.c",
    "sounds.c",
    "statdump.c",
    "st_lib.c",
    "st_stuff.c",
    "s_sound.c",
    "tables.c",
    "v_video.c",
    "wi_stuff.c",
    "w_checksum.c",
    "w_file.c",
    "w_main.c",
    "w_wad.c",
    "z_zone.c",
    "w_file_stdc.c",
    "i_input.c",
    "i_video.c",
    "doomgeneric.c",
    "doomgeneric_tg.c",
    "tg_libc_shim.c",
];
