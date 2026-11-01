struct Questions {
    p_persistent: f64,
    p_active: f64,
}

fn classify(questions: &Questions) -> &'static str {
    let persistent = questions.p_persistent >= 0.80;
    let active = questions.p_active >= 0.80;

    match (persistent, active) {
        (true, true) => "both strong",
        (true, false) => "persistence only",
        (false, true) => "activity only",
        (false, false) => "neither",
    }
}

fn main() {
    let fixture1 = Questions {
        p_persistent: 0.85,
        p_active: 0.90,
    };

    let fixture2 = Questions {
        p_persistent: 0.85,
        p_active: 0.30,
    };

    let fixture3 = Questions {
        p_persistent: 0.30,
        p_active: 0.90,
    };

    println!("{}", classify(&fixture1));
    println!("{}", classify(&fixture2));
    println!("{}", classify(&fixture3));
}

#[cfg(test)]
mod tests {
    #[test]
    fn probabilities_can_sum_to_more_than_one() {
        let p_persistent = 0.85;
        let p_active = 0.90;

        assert!(p_persistent + p_active > 1.0);
    }
}
