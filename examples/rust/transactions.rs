//! Transaction example: commit, rollback, and savepoints.
//!
//! Run with: `cargo run -p grafeo-examples --bin transactions`

use grafeo::GrafeoDB;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let db = GrafeoDB::new_in_memory();
    let mut session = db.session();

    // Set up two bank accounts as nodes with a balance property
    session.execute("INSERT (:Account {owner: 'Alix', balance: 1000})")?;
    session.execute("INSERT (:Account {owner: 'Gus', balance: 500})")?;

    print_balances(&session, "Initial balances")?;

    // ── Act 1: Committed transfer ─────────────────────────────────
    // Begin a transaction. All changes are isolated until commit.
    session.begin_transaction()?;

    // Transfer 200 from Alix to Gus
    session.execute(
        "MATCH (a:Account {owner: 'Alix'})
         SET a.balance = a.balance - 200",
    )?;
    session.execute(
        "MATCH (a:Account {owner: 'Gus'})
         SET a.balance = a.balance + 200",
    )?;

    // Commit makes the changes permanent
    session.commit()?;
    print_balances(&session, "After committed transfer (Alix -> Gus, 200)")?;

    // ── Act 2: Rolled-back transfer ───────────────────────────────
    // Start another transaction, but this time undo everything.
    session.begin_transaction()?;

    // Attempt to transfer 300 from Gus to Alix
    session.execute(
        "MATCH (a:Account {owner: 'Gus'})
         SET a.balance = a.balance - 300",
    )?;
    session.execute(
        "MATCH (a:Account {owner: 'Alix'})
         SET a.balance = a.balance + 300",
    )?;

    // Oops, rollback discards all changes in this transaction
    session.rollback()?;
    print_balances(&session, "After rollback (no change)")?;

    // ── Act 3: Savepoints for partial rollback ────────────────────
    // Savepoints let you undo part of a transaction while keeping
    // the rest.
    session.begin_transaction()?;

    // Step 1: create a new account (will be kept)
    session.execute("INSERT (:Account {owner: 'Vincent', balance: 750})")?;

    // Mark this point so we can come back to it
    session.savepoint("after_vincent")?;

    // Step 2: create another account (will be undone)
    session.execute("INSERT (:Account {owner: 'Jules', balance: 250})")?;

    // Undo step 2, but keep step 1
    session.rollback_to_savepoint("after_vincent")?;

    // Commit: only Vincent's account persists
    session.commit()?;

    // Verify: Vincent exists, Jules does not
    let vincent: i64 = session
        .execute("MATCH (a:Account {owner: 'Vincent'}) RETURN COUNT(a)")?
        .scalar()?;
    let jules: i64 = session
        .execute("MATCH (a:Account {owner: 'Jules'}) RETURN COUNT(a)")?
        .scalar()?;

    println!("After savepoint rollback and commit:");
    println!("  Vincent exists: {}", vincent == 1);
    println!("  Jules exists:   {}", jules == 1);

    println!("\nDone!");
    Ok(())
}

/// Helper to print all account balances
fn print_balances(
    session: &grafeo::Session,
    label: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let result = session.execute(
        "MATCH (a:Account)
         RETURN a.owner, a.balance
         ORDER BY a.owner",
    )?;

    println!("\n{label}:");
    for row in result.iter() {
        let owner = row[0].as_str().unwrap_or("?");
        let balance = row[1].as_int64().unwrap_or(0);
        println!("  {:<10} ${}", owner, balance);
    }
    Ok(())
}
