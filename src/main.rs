
#![no_std]
#![no_main]

use bleps::{
    ad_structure::{
        create_advertising_data, AdStructure, BR_EDR_NOT_SUPPORTED, LE_GENERAL_DISCOVERABLE,
    },
    attribute_server::{
        AttributeServer, NotificationData, WorkResult},
    gatt, Ble, HciConnector,
};
use esp_backtrace as _;
use esp_println::println;
use esp_wifi::{ble::controller::BleConnector, initialize, EspWifiInitFor};
use hal::{
    adc::{AdcConfig, Attenuation, ADC, ADC1},
    clock::ClockControl, 
    peripherals::*, 
    prelude::*, 
    Rng, 
    IO, 
    ledc::{
        channel::{self, ChannelIFace},
        timer::{self, TimerIFace},
        HighSpeed,
        LEDC,
    },
    Delay,
};

#[entry]
fn main() -> ! {
    let peripherals = Peripherals::take();

    let mut mac = [0; 6];
    mac[0..4].copy_from_slice(&peripherals.RNG.data.read().bits().to_le_bytes());
    hal::efuse::Efuse::set_mac_address(mac).unwrap();

    let system = peripherals.SYSTEM.split();
    let clocks = ClockControl::max(system.clock_control).freeze();

    let timer = hal::timer::TimerGroup::new(peripherals.TIMG1, &clocks).timer0;
    let init = initialize(
        EspWifiInitFor::Ble,
        timer,
        Rng::new(peripherals.RNG),
        system.radio_clock_control,
        &clocks,
    )
    .unwrap();

    let io = IO::new(peripherals.GPIO, peripherals.IO_MUX);
    let mut bluetooth = peripherals.BT;

    // PWM config
    let pin26 = io.pins.gpio26.into_push_pull_output();

    let pwm = LEDC::new(peripherals.LEDC, &clocks);
    let mut hstimer0 = pwm.get_timer::<HighSpeed>(timer::Number::Timer0);

    hstimer0
        .configure(timer::config::Config {
            duty: timer::config::Duty::Duty15Bit,
            clock_source: timer::HSClockSource::APBClk,
            frequency: 2u32.kHz(),
        })
        .unwrap();

    let mut channel0 = pwm.get_channel(channel::Number::Channel0, pin26);
    channel0
        .configure(channel::config::Config {
            timer: &hstimer0,
            duty_pct: 10,
            pin_config: channel::config::PinConfig::PushPull,
        })
        .unwrap();

    // ADC configuration

    let analog = peripherals.SENS.split();

    let mut adc1_config = AdcConfig::new();
    let mut pin32 = adc1_config.enable_pin(io.pins.gpio32.into_analog(), Attenuation::Attenuation11dB);
    let mut adc1 = ADC::<ADC1>::adc(analog.adc1, adc1_config).unwrap();

    let mut delay = Delay::new(&clocks);

    loop {
        channel0.set_duty(0).unwrap();

        // BLE configuration
        let connector = BleConnector::new(&init, &mut bluetooth);
        let hci = HciConnector::new(connector, esp_wifi::current_millis);
        let mut ble = Ble::new(&hci);

        println!("{:?}", ble.init());
        println!("{:?}", ble.cmd_set_le_advertising_parameters());
        println!(
            "{:?}",
            ble.cmd_set_le_advertising_data(
                create_advertising_data(&[
                    AdStructure::Flags(LE_GENERAL_DISCOVERABLE | BR_EDR_NOT_SUPPORTED),
                    AdStructure::ServiceUuids16(&[Uuid::Uuid16(0x1809)]),
                    AdStructure::CompleteLocalName("ESP-walk-machine"),
                ])
                .unwrap()
            )
        );
        println!("{:?}", ble.cmd_set_le_advertise_enable(true));

        println!("started advertising");

        let mut adc_read = |_offset: usize, data: &mut [u8]| {
            data[..3].copy_from_slice(&b"adc"[..]);
            3
        };
        let mut update_pwm = |offset: usize, data: &[u8]| {
            println!("RECEIVED: {} {:?}", offset, data);

            match data {
                [duty] if *duty <= 100 => {
                    channel0.set_duty(*duty).unwrap();
                }
                _=> println!("invalid data")
            }
        };

        gatt!([service {
            uuid: "937312e0-2354-11eb-9f10-fbc30a62cf38",
            characteristics: [
                characteristic {
                    uuid: "937312e0-2354-11eb-9f10-fbc30a62cf37",
                    write: update_pwm,
                    name: "pwm_control",
                },
                characteristic {
                    uuid: "937312e0-2354-11eb-9f10-fbc30a62cf36",
                    read: adc_read,
                    name: "pwm_control",
                },
            ],
        },]);

        let mut srv = AttributeServer::new(&mut ble, &mut gatt_attributes);

        loop {
            let notification = None;

            let pin32_value: u16 = nb::block!(adc1.read(&mut pin32)).unwrap();
            println!("Read adc = {}", pin32_value);
            delay.delay_ms(500u32);

            match srv.do_work_with_notification(notification) {
                Ok(res) => {
                    if let WorkResult::GotDisconnected = res {
                        break;
                    }
                }
                Err(err) => {
                    println!("{:?}", err);
                    break;
                }
            }
        }
    }
}