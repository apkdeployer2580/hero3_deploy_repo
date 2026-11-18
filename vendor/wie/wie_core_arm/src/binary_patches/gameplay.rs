use crate::{ArmCore, engine::ArmRegister, function::JumpTo};
use wie_util::{ByteWrite, Result, read_generic};

pub async fn respawn(core: &mut ArmCore, exit_pc: u32) -> Result<JumpTo> {
    let addr = core.inner.lock().engine.reg_read(ArmRegister::R6);
    let timer: u16 = read_generic(core, addr)?;
    let threshold: i16 = read_generic(core, addr - 2)?;
    let next = wie_util::gameplay::respawn_counter(timer, threshold, wie_util::gameplay::respawn_rate());
    core.inner.lock().engine.reg_write(ArmRegister::R3, u32::from(next));
    Ok(JumpTo(exit_pc))
}

pub async fn exp(core: &mut ArmCore, exit_pc: u32) -> Result<JumpTo> {
    let (stats, enemy, base) = {
        let inner = core.inner.lock();
        (
            inner.engine.reg_read(ArmRegister::R4),
            inner.engine.reg_read(ArmRegister::R5),
            inner.engine.reg_read(ArmRegister::R3) as i16,
        )
    };
    let old: u32 = read_generic(core, stats + 0x30)?;
    let bonus: i16 = read_generic(core, enemy)?;
    let rate = wie_util::gameplay::exp_rate();
    let gain = wie_util::gameplay::kill_exp(base, bonus, rate);
    let total = if rate == 1 {
        old.wrapping_add(gain as u32)
    } else {
        (i64::from(old) + gain).clamp(0, i64::from(i32::MAX)) as u32
    };
    core.write_bytes(stats + 0x30, &total.to_le_bytes())?;
    let r0: u32 = read_generic(core, 0x4d408)?;
    let mut inner = core.inner.lock();
    inner.engine.reg_write(ArmRegister::R0, r0);
    inner.engine.reg_write(ArmRegister::R1, total);
    inner.engine.reg_write(ArmRegister::R2, 0);
    inner.engine.reg_write(ArmRegister::R3, bonus as i32 as u32);
    Ok(JumpTo(exit_pc))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[futures_test::test]
    async fn hooks_keep_the_default_and_scale_without_halfword_overflow() -> Result<()> {
        let mut core = ArmCore::new(false, None)?;
        core.map(0x10000, 0x1000)?;
        core.map(0x4d000, 0x1000)?;
        core.write_bytes(0x4d408, &0x1503ab0u32.to_le_bytes())?;
        core.write_bytes(0x10030, &1000u32.to_le_bytes())?;
        core.write_bytes(0x10200, &7i16.to_le_bytes())?;
        {
            let mut inner = core.inner.lock();
            inner.engine.reg_write(ArmRegister::R4, 0x10000);
            inner.engine.reg_write(ArmRegister::R5, 0x10200);
            inner.engine.reg_write(ArmRegister::R3, 101);
        }
        wie_util::gameplay::set_exp_rate(1);
        exp(&mut core, 0x4d3f3).await?;
        assert_eq!(read_generic::<u32, _>(&mut core, 0x10030)?, 1108);
        core.inner.lock().engine.reg_write(ArmRegister::R3, 30000);
        wie_util::gameplay::set_exp_rate(6);
        exp(&mut core, 0x4d3f3).await?;
        assert_eq!(read_generic::<u32, _>(&mut core, 0x10030)?, 181150);
        core.write_bytes(0x10372, &300i16.to_le_bytes())?;
        core.write_bytes(0x10374, &298u16.to_le_bytes())?;
        core.inner.lock().engine.reg_write(ArmRegister::R6, 0x10374);
        wie_util::gameplay::set_respawn_rate(6);
        respawn(&mut core, 0x3a713).await?;
        assert_eq!(core.inner.lock().engine.reg_read(ArmRegister::R3), 300);
        core.write_bytes(0x10374, &65535u16.to_le_bytes())?;
        respawn(&mut core, 0x3a713).await?;
        assert_eq!(core.inner.lock().engine.reg_read(ArmRegister::R3), 65535);
        wie_util::gameplay::set_exp_rate(1);
        wie_util::gameplay::set_respawn_rate(1);
        Ok(())
    }
}
