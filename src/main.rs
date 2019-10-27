use clap;
use spectrum::audio;
use spectrum::led;
use std::str::FromStr;
fn main() {
    let args = parse_args();
    let brightness = 
        f32::from_str(args.value_of("brightness").unwrap()).unwrap(); // neither unwrap should ever fail
    let pp = audio::PendingProducer::new_jack(1024).unwrap();
    let mut con = led::Controller::new(18, 300, false, brightness);
    match args.value_of("scaling_alg").unwrap() {
        "linear" => con = con.set_alg(led::Algorithm::Linear),
        "quadratic" => con = con.set_alg(led::Algorithm::Quadratic),
        _ => panic!("Unimplemented value for scaling_alg")
    }
    con.display(pp);    
    
}

fn parse_args<'a>() -> clap::ArgMatches<'a> {
    clap::App::new("spectrum")
        .author("curtismaves@gmail.com")
        .version("0.1.0")
        .about("This program takes ")
        .arg(
           clap::Arg::with_name("brightness")
                .short("b")
                .long("brightness")
                .value_name("brightness")
                .help("Set the brightness of the LEDs. It should be between (0, 1]")
                .takes_value(true)
                .default_value("1.0")
                .validator(|s| {
                    match f32::from_str(&s) {
                        Ok(f) => if f <= 0.0 || f > 1.0 {
                            Err("Brightness must be be between (0, 1]".to_string())
                        } else {
                            Ok(())
                        }
                        Err(_) => Err("Brightness should be a float".to_string())
                    }
                })
        )
        .arg(
            clap::Arg::with_name("scaling_alg")
                .short("s")
                .long("scaling")
                .takes_value(true)
                .value_name("ALGORITHM")
                .help("Sets the alogorithm used scale audio input for the program to visual.")
                .possible_values(&["linear", "quadratic"])
                .default_value("quadratic")
        )
        .get_matches()
}
